#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    collections::BTreeSet,
    io::ErrorKind,
    net::TcpListener,
    time::{Duration, Instant},
};

use lazydb::{
    connections::DatabaseConnector,
    db::{
        DatabaseConnection, ErrorCategory,
        mongodb::{MongoAdapter, transaction::MongoTransactionOutcome},
    },
    profile::{
        ConnectionProfile, DatabaseKind, MongoSeed, SshOptions, SslMode, import_connection_url,
    },
};
use mongodb::{
    Collection,
    bson::{Document, doc},
};
use secrecy::SecretString;
use support::{FixtureManifest, fixture, kill_managed_ssh_tunnels};

fn ssh_options(fixture: &FixtureManifest) -> SshOptions {
    SshOptions {
        host: fixture.ssh.host.clone(),
        port: Some(fixture.ssh.port),
        user: Some(fixture.ssh.user.clone()),
        identity_file: Some(fixture.ssh.client_key.clone()),
        config_file: Some(fixture.ssh.config_file.clone()),
        known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
        proxy_jump: None,
    }
}

fn mongo_profile(fixture: &FixtureManifest, name: &str, username: &str) -> ConnectionProfile {
    let mut profile = import_connection_url(&fixture.mongo.url, Some(name))
        .unwrap()
        .profile;
    assert_eq!(profile.ssl_mode, SslMode::Require);
    profile.ssl_mode = SslMode::VerifyFull;
    profile.user = Some(username.to_owned());
    profile.ssh = Some(ssh_options(fixture));
    let options = profile
        .mongodb_options
        .as_ref()
        .expect("MongoDB options were not imported");
    assert!(options.uri_options.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("authSource") && value == fixture.mongo.auth_source()
    }));
    assert!(options.uri_options.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("retryWrites") && value.eq_ignore_ascii_case("false")
    }));
    profile
}

fn expected_hosts(fixture: &FixtureManifest) -> BTreeSet<String> {
    fixture
        .mongo
        .require_members()
        .iter()
        .map(|member| format!("{}:{}", member.host, member.port))
        .collect()
}

async fn connect_mongo(
    fixture: &FixtureManifest,
    name: &str,
) -> (DatabaseConnector, DatabaseConnection) {
    let username = fixture.mongo.username();
    let password = SecretString::from(fixture.mongo.password());
    let profile = mongo_profile(fixture, name, &username);
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .unwrap_or_else(|error| panic!("connect MongoDB through managed SSH SOCKS: {error:?}"));
    (connector, connection)
}

fn mongo_adapter(connection: &DatabaseConnection) -> &MongoAdapter {
    match connection {
        DatabaseConnection::MongoDb(adapter) => adapter,
        _ => panic!("MongoDB profile used the wrong adapter"),
    }
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mongo_replica_set_tls_discovery_and_scram_use_socks() {
    let fixture = fixture();
    let username = fixture.mongo.username();
    let (connector, connection) = connect_mongo(&fixture, "ssh-mongo-replica-set").await;
    let adapter = mongo_adapter(&connection);

    let admin = adapter.client().database("admin");
    let hello = admin.run_command(doc! { "hello": 1 }).await.unwrap();
    assert_eq!(hello.get_str("setName").unwrap(), fixture.mongo.replica_set);
    assert!(hello.get_bool("isWritablePrimary").unwrap());
    let hosts = hello
        .get_array("hosts")
        .unwrap()
        .iter()
        .filter_map(|host| host.as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let expected = expected_hosts(&fixture);
    assert_eq!(hosts, expected);
    assert!(expected.contains(hello.get_str("primary").unwrap()));

    let options = adapter
        .client()
        .database("admin")
        .run_command(doc! { "getCmdLineOpts": 1 })
        .await
        .unwrap();
    let parsed = options.get_document("parsed").unwrap();
    let tls = parsed
        .get_document("net")
        .unwrap()
        .get_document("tls")
        .unwrap();
    assert_eq!(tls.get_str("mode").unwrap(), "requireTLS");
    let security = parsed.get_document("security").unwrap();
    assert_eq!(security.get_str("authorization").unwrap(), "enabled");
    assert!(!security.get_str("keyFile").unwrap().is_empty());

    let status = adapter
        .client()
        .database("admin")
        .run_command(doc! { "connectionStatus": 1 })
        .await
        .unwrap();
    let authenticated = status
        .get_document("authInfo")
        .unwrap()
        .get_array("authenticatedUsers")
        .unwrap();
    assert!(authenticated.iter().any(|entry| {
        entry
            .as_document()
            .and_then(|entry| entry.get_str("user").ok())
            == Some(username.as_str())
    }));

    let collection = adapter
        .client()
        .database(&fixture.mongo.database)
        .collection::<Document>("ssh_fixture");
    collection
        .insert_one(doc! { "_id": "socks", "value": "managed" })
        .await
        .unwrap();
    let databases = adapter.list_database_names().await.unwrap();
    assert!(databases.contains(&fixture.mongo.database));
    assert_eq!(adapter.probe().await.unwrap().kind, DatabaseKind::MongoDb);
    collection.drop().await.unwrap();

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mongo_explicit_seeds_use_all_three_members() {
    let fixture = fixture();
    let username = fixture.mongo.username();
    let password = SecretString::from(fixture.mongo.password());
    let mut profile = mongo_profile(&fixture, "ssh-mongo-explicit-seeds", &username);
    let seeds = fixture
        .mongo
        .require_members()
        .iter()
        .map(|member| MongoSeed {
            host: member.host.clone(),
            port: Some(member.port),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        seeds
            .iter()
            .map(|seed| format!("{}:{:?}", seed.host, seed.port))
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    profile.mongodb_options.as_mut().unwrap().seeds = seeds;

    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect MongoDB with all explicit replica-set seeds");
    let adapter = mongo_adapter(&connection);
    let admin = adapter.client().database("admin");
    let hello = admin.run_command(doc! { "hello": 1 }).await.unwrap();
    assert_eq!(hello.get_str("setName").unwrap(), fixture.mongo.replica_set);
    assert_eq!(
        hello
            .get_array("hosts")
            .unwrap()
            .iter()
            .filter_map(|host| host.as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>(),
        expected_hosts(&fixture)
    );

    let collection = adapter
        .client()
        .database(&fixture.mongo.database)
        .collection::<Document>("ssh_explicit_seeds");
    collection
        .insert_one(doc! { "_id": "seeds", "value": 3 })
        .await
        .unwrap();
    collection.drop().await.unwrap();

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mongo_wrong_password_is_rejected() {
    let fixture = fixture();
    let username = fixture.mongo.username();
    let profile = mongo_profile(&fixture, "ssh-mongo-wrong-password", &username);
    let wrong_password = SecretString::from("not-the-fixture-password");
    let connector = DatabaseConnector::new();
    let error = connector
        .connect(&profile, Some(&wrong_password))
        .await
        .expect_err("MongoDB accepted an invalid SCRAM password");
    assert_eq!(error.category, ErrorCategory::Authentication, "{error:?}");
    assert!(!error.message.contains("not-the-fixture-password"));
    connector.shutdown().await;
}

async fn wait_for_new_primary(
    adapter: &MongoAdapter,
    old_primary: &str,
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let result = adapter
            .client()
            .database("admin")
            .run_command(doc! { "hello": 1 })
            .await;
        match result {
            Ok(hello) if hello.get_bool("isWritablePrimary").unwrap_or(false) => {
                if let Ok(primary) = hello.get_str("primary")
                    && primary != old_primary
                {
                    return primary.to_owned();
                }
            }
            Ok(_) => {}
            Err(error) if Instant::now() >= deadline => {
                panic!("MongoDB did not discover a new primary after stepdown: {error:?}")
            }
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            panic!("MongoDB did not discover a new primary after stepping down {old_primary}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn count_eventually(
    collection: &Collection<Document>,
    filter: Document,
    timeout: Duration,
) -> u64 {
    let deadline = Instant::now() + timeout;
    loop {
        match collection.count_documents(filter.clone()).await {
            Ok(count) => return count,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("MongoDB count did not recover after failover: {error:?}"),
        }
    }
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mongo_stepdown_discovers_new_primary_without_replaying_uncertain_write() {
    let fixture = fixture();
    assert!(
        fixture
            .mongo
            .url
            .to_ascii_lowercase()
            .contains("retrywrites=false")
    );
    let (connector, connection) = connect_mongo(&fixture, "ssh-mongo-stepdown-failover").await;
    let adapter = mongo_adapter(&connection);
    let collection = adapter
        .client()
        .database(&fixture.mongo.database)
        .collection::<Document>("ssh_failover");
    collection.delete_many(doc! {}).await.unwrap();
    collection
        .insert_one(doc! { "_id": "baseline", "value": "before-stepdown" })
        .await
        .unwrap();

    let admin = adapter.client().database("admin");
    let hello = admin.run_command(doc! { "hello": 1 }).await.unwrap();
    let old_primary = hello.get_str("primary").unwrap().to_owned();
    assert!(hello.get_bool("isWritablePrimary").unwrap());

    let step_down = admin.run_command(doc! {
        "replSetStepDown": 30,
        "secondaryCatchUpPeriodSecs": 5,
        "force": true,
    });
    let uncertain_write = collection.insert_one(doc! {
        "_id": "during-stepdown",
        "value": "single-attempt",
    });
    let (step_down_result, uncertain_write_result) = tokio::join!(step_down, uncertain_write);
    if let Err(error) = step_down_result {
        let rendered = error.to_string().to_ascii_lowercase();
        assert!(
            !rendered.contains("not authorized") && !rendered.contains("authentication"),
            "replSetStepDown failed: {error:?}"
        );
    }

    let new_primary = wait_for_new_primary(adapter, &old_primary, Duration::from_secs(45)).await;
    assert_ne!(new_primary, old_primary);
    let sentinel_count = count_eventually(
        &collection,
        doc! { "_id": "during-stepdown" },
        Duration::from_secs(15),
    )
    .await;
    assert!(
        sentinel_count <= 1,
        "uncertain write was replayed: count={sentinel_count}, result={uncertain_write_result:?}"
    );

    collection
        .insert_one(doc! { "_id": "after-failover", "value": "new-primary" })
        .await
        .expect("single post-failover write");
    assert_eq!(
        count_eventually(
            &collection,
            doc! { "_id": "after-failover" },
            Duration::from_secs(5),
        )
        .await,
        1
    );
    collection.drop().await.unwrap();

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mongo_transaction_read_and_abort_use_socks() {
    let fixture = fixture();
    let (connector, connection) = connect_mongo(&fixture, "ssh-mongo-transaction").await;
    let adapter = mongo_adapter(&connection);
    let collection = adapter
        .client()
        .database(&fixture.mongo.database)
        .collection::<Document>("ssh_transactions");
    collection.delete_many(doc! {}).await.unwrap();
    collection
        .insert_one(doc! { "_id": "visible", "value": 1 })
        .await
        .unwrap();

    let mut transaction = adapter.begin_session_transaction().await.unwrap();
    let reply = transaction
        .run_command(
            &fixture.mongo.database,
            doc! { "find": "ssh_transactions", "filter": { "_id": "visible" } },
        )
        .await
        .unwrap();
    let batch = reply
        .get_document("cursor")
        .unwrap()
        .get_array("firstBatch")
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].as_document().unwrap().get_i32("value").unwrap(), 1);
    assert_eq!(
        transaction.abort().await.unwrap(),
        MongoTransactionOutcome::Aborted
    );

    assert_eq!(
        collection
            .count_documents(doc! { "_id": "visible" })
            .await
            .unwrap(),
        1
    );
    collection.drop().await.unwrap();

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn crashed_mongo_tunnel_fails_closed_before_reusing_its_loopback_port() {
    let fixture = fixture();
    let (connector, connection) = connect_mongo(&fixture, "ssh-mongo-tunnel-crash").await;
    let adapter = mongo_adapter(&connection);
    let session = adapter.client().start_session().await.unwrap();
    let released = kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    let port = released
        .iter()
        .find_map(|tunnel| tunnel.local_port)
        .expect("managed MongoDB tunnel did not expose a loopback port");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match adapter.probe().await {
            Err(error) if error.code.as_deref() == Some("ssh_tunnel_exited") => {
                assert_eq!(error.category, ErrorCategory::Network);
                break;
            }
            Err(error) => assert!(
                Instant::now() < deadline,
                "MongoDB route did not observe the tunnel crash: {error:?}"
            ),
            Ok(_) => assert!(
                Instant::now() < deadline,
                "MongoDB route did not observe the tunnel crash"
            ),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            adapter
                .client()
                .database("admin")
                .run_command(doc! { "ping": 1 }),
        )
        .await;
        if result.is_ok_and(|result| {
            result.is_err_and(|error| matches!(*error.kind, mongodb::error::ErrorKind::Shutdown))
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "MongoDB background client was not shut down with a live session"
        );
    }
    drop(session);
    let listener = bind_released_loopback(port).await;
    let error = adapter
        .probe()
        .await
        .expect_err("MongoDB reused a loopback endpoint after tunnel failure");
    assert_eq!(error.code.as_deref(), Some("ssh_tunnel_exited"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "MongoDB connected to a replacement process on the old tunnel port"
    );

    connection.close().await;
    connector.shutdown().await;
}

async fn bind_released_loopback(port: u16) -> TcpListener {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).unwrap();
                return listener;
            }
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("could not claim the released SSH port {port}: {error}"),
        }
    }
}
