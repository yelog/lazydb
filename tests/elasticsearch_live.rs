use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogSearchObjectScope,
            CatalogSearchRequest, CatalogTarget, ObjectGroup,
        },
        query::QueryOutcome,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, PageSize, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::{ElasticsearchAuth, ElasticsearchConnectionOptions, import_connection_url},
};
use secrecy::SecretString;
use serde_json::{Value, json};

fn response(result: &QueryOutcome) -> Value {
    serde_json::from_str(&result.result_sets.last().unwrap().rows[0][0].clipboard_text()).unwrap()
}

#[tokio::test]
#[ignore = "requires an isolated Elasticsearch server with security enabled and an admin user"]
async fn live_catalog_preview_queries_and_permissions() {
    let url = std::env::var("LAZYDB_TEST_ELASTICSEARCH_URL")
        .expect("set LAZYDB_TEST_ELASTICSEARCH_URL to an isolated test server");
    let imported = import_connection_url(&url, None).unwrap();
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let index = format!("lazydb-test-{suffix}");
    let role = format!("lazydb_test_{suffix}");
    let user = role.clone();
    let password = uuid::Uuid::new_v4().to_string();

    // Always remove only this run's randomly named resources, even after an assertion fails.
    let result = AssertUnwindSafe(async {
        let server = connection.probe().await.unwrap();
        assert!(!server.version.is_empty());
        eprintln!("live Elasticsearch version: {}", server.version);
        let mapping = json!({"mappings":{"properties":{
            "title":{"type":"text","fields":{"keyword":{"type":"keyword"}}},
            "n":{"type":"integer"},
            "author":{"properties":{"name":{"type":"keyword"}}}
        }}});
        let created = connection.execute(&format!("PUT /{index}\n{mapping}")).await.unwrap();
        assert_eq!(response(&created)["acknowledged"], true);
        let mut bulk = format!("POST /{index}/_bulk?refresh=wait_for\n");
        for n in 0..23 {
            bulk.push_str(&format!("{}\n{}\n", json!({"index":{"_id":n.to_string()}}),
                json!({"title":format!("Rust book {n}"),"n":n,"author":{"name":"Ada"},"_hit":"source-hit"})));
        }
        let inserted = connection.execute(&bulk).await.unwrap();
        assert_eq!(response(&inserted)["errors"], false, "{}", response(&inserted));
        assert_eq!(response(&inserted)["items"].as_array().unwrap().len(), 23);

        let identity = ConnectionIdentity { profile_id: imported.profile.id, generation: 1 };
        let request = |target| CatalogRequest {
            key: CatalogRequestKey { connection: identity, catalog_epoch: 1, request_id: 1, target, cursor: None },
            scope: imported.profile.catalog_scope.clone(),
            page_size: 500,
        };
        let page = connection.load_catalog_page(&request(CatalogTarget::Objects {
            schema: CatalogId::new(imported.profile.id, CatalogKind::Schema, ["default", "default"]),
            group: ObjectGroup::Tables,
        })).await.unwrap();
        let relation = page.entries.iter().find(|entry| entry.qualified_name.object == index).unwrap().id.clone();
        assert!(page.entries.iter().all(|entry| !entry.qualified_name.object.starts_with('.')));
        let fields = connection.load_catalog_page(&request(CatalogTarget::RelationChildren { relation: relation.clone() })).await.unwrap();
        for name in ["title", "title.keyword", "author.name"] {
            assert!(fields.entries.iter().any(|entry| entry.qualified_name.object == name), "missing {name}");
        }
        let found = connection.search_catalog(&CatalogSearchRequest {
            connection: identity, session_id: 1, generation: 1, query: suffix.clone(),
            scope: imported.profile.catalog_scope.clone(), object_scope: CatalogSearchObjectScope::RelationsOnly, limit: 100,
        }).await.unwrap();
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].entry.id, relation);

        let mut ids = std::collections::HashSet::new();
        for (offset, rows, next) in [(0, 10, true), (10, 10, true), (20, 3, false)] {
            let preview = connection.preview_relation(&relation, &RelationPreviewOptions::default(), PageRequest::at(PageSize::Ten, offset)).await.unwrap();
            assert_eq!(preview.pagination.total, TotalRows::Exact(23));
            assert_eq!(preview.pagination.has_next, next);
            assert_eq!(preview.result.result_sets[0].rows.len(), rows);
            for row in &preview.result.result_sets[0].rows {
                assert!(ids.insert(row[1].clipboard_text()));
            }
        }
        assert_eq!(ids.len(), 23);
        let mut last_page = PageRequest::first(PageSize::Ten);
        last_page.resolve_total = true;
        let last = connection.preview_relation(&relation, &Default::default(), last_page).await.unwrap();
        assert_eq!(last.pagination.offset, 20);
        assert_eq!(last.result.result_sets[0].rows.len(), 3);

        let search = connection.execute(&format!("POST /{index}/_search\n{}", json!({
            "size":2, "query":{"match":{"title":"Rust"}}, "sort":["n"],
            "highlight":{"fields":{"title":{}}},
            "aggs":{"authors":{"terms":{"field":"author.name"}}}
        }))).await.unwrap();
        let metadata = response(&search);
        assert_eq!(metadata["hits"]["total"]["value"], 23);
        assert_eq!(metadata["aggregations"]["authors"]["buckets"][0]["doc_count"], 23);
        let columns = &search.result_sets[0].columns;
        let hit_column = columns.iter().position(|column| column.name == "_hit").unwrap();
        let hit: Value = serde_json::from_str(&search.result_sets[0].rows[0][hit_column].clipboard_text()).unwrap();
        assert!(hit["highlight"]["title"].is_array());
        assert_eq!(hit["sort"][0], 0);
        let source_hit_column = columns.iter().position(|column| column.name == "_source._hit").unwrap();
        assert_eq!(search.result_sets[0].rows[0][source_hit_column].clipboard_text(), "source-hit");

        let multi = connection.execute(&format!("POST /{index}/_msearch\n{{}}\n{{\"size\":0}}\n{{}}\n{{\"query\":{{\"term\":{{\"n\":999}}}}}}" )).await.unwrap();
        let multi = response(&multi);
        assert_eq!(multi["responses"][0]["hits"]["total"]["value"], 23);
        assert_eq!(multi["responses"][1]["hits"]["total"]["value"], 0);
        let conflict = connection.execute(&format!("PUT /{index}/_create/0\n{{\"n\":99}}")).await.unwrap_err();
        assert_eq!(conflict.category, ErrorCategory::Constraint);

        let partial_bulk = connection.execute(&format!("POST /{index}/_bulk?refresh=wait_for\n{{\"create\":{{\"_id\":\"0\"}}}}\n{{\"n\":99}}\n{{\"index\":{{\"_id\":\"partial-success\"}}}}\n{{\"n\":24}}" )).await.unwrap();
        assert!(partial_bulk.stats.response_error.as_deref().unwrap().contains("bulk: 1 of 2"));
        let bulk_json = response(&partial_bulk);
        assert_eq!(bulk_json["items"][0]["create"]["status"], 409);
        assert_eq!(bulk_json["items"][1]["index"]["status"], 201);
        assert_eq!(response(&connection.execute(&format!("GET /{index}/_doc/partial-success")).await.unwrap())["_source"]["n"], 24);
        let partial_search = connection.execute(&format!("POST /{index}/_msearch\n{{}}\n{{\"size\":0}}\n{{\"index\":\"lazydb-missing-{suffix}\"}}\n{{\"size\":0}}" )).await.unwrap();
        assert!(partial_search.stats.response_error.as_deref().unwrap().contains("msearch: 1 of 2"));
        assert!(response(&partial_search)["responses"][1]["error"].is_object());

        let mut read_only = imported.profile.clone();
        read_only.read_only = true;
        let reader = DatabaseConnection::connect(&read_only, imported.transient_password.as_ref()).await.unwrap();
        reader.execute(&format!("GET /{index}/_count")).await.unwrap();
        let denied = reader.execute(&format!("DELETE /{index}")).await.unwrap_err();
        assert_eq!(denied.code.as_deref(), Some("profile_read_only"));
        reader.close().await;

        // The connection probe needs cluster monitor; index-monitor and writes stay denied.
        connection.execute(&format!("PUT /_security/role/{role}\n{}", json!({"cluster":["monitor"],"indices":[{"names":[index],"privileges":["read","view_index_metadata"]}]}))).await.unwrap();
        connection.execute(&format!("PUT /_security/user/{user}\n{}", json!({"password":password,"roles":[role]}))).await.unwrap();
        let mut restricted = imported.profile.clone();
        restricted.user = Some(user.clone());
        let restricted = DatabaseConnection::connect(&restricted, Some(&SecretString::from(password.clone()))).await.unwrap();
        restricted.execute(&format!("GET /{index}/_count")).await.unwrap();
        let denied = restricted.execute(&format!("PUT /{index}/_doc/denied\n{{\"n\":99}}")).await.unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Permission);
        let denied = restricted.execute("GET /_cat/indices?format=json").await.unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Permission);
        restricted.close().await;
        let denied = DatabaseConnection::connect(&imported.profile, Some(&SecretString::from("wrong-test-password"))).await.unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Authentication);
    }).catch_unwind().await;

    let mut cleanup_errors = Vec::new();
    for path in [
        format!("/{index}"),
        format!("/_security/user/{user}"),
        format!("/_security/role/{role}"),
    ] {
        if let Err(error) = connection.execute(&format!("DELETE {path}")).await
            && !matches!(
                error.code.as_deref(),
                Some("index_not_found_exception" | "404")
            )
        {
            cleanup_errors.push(error);
        }
    }
    connection.close().await;
    if let Err(panic) = result {
        if !cleanup_errors.is_empty() {
            eprintln!("resource cleanup errors: {cleanup_errors:?}");
        }
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "resource cleanup errors: {cleanup_errors:?}"
    );
}

#[tokio::test]
#[ignore = "requires the Compose HTTPS fixture with a localhost-only certificate"]
async fn live_https_custom_ca_preserves_chain_and_hostname_verification() {
    let url = std::env::var("LAZYDB_TEST_ELASTICSEARCH_TLS_URL")
        .expect("set LAZYDB_TEST_ELASTICSEARCH_TLS_URL to the localhost HTTPS fixture");
    let mut imported = import_connection_url(&url, None).unwrap();
    assert_eq!(imported.profile.host.as_deref(), Some("localhost"));
    let path =
        std::path::PathBuf::from(std::env::var("LAZYDB_TEST_ELASTICSEARCH_TLS_CERT").expect(
            "set LAZYDB_TEST_ELASTICSEARCH_TLS_CERT to the fixture's public PEM certificate",
        ));
    imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::Basic,
        ca_cert_path: Some(path.clone()),
    });
    let trusted =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    assert!(!trusted.probe().await.unwrap().version.is_empty());
    trusted.close().await;

    // A PEM bundle must work as well as a single certificate.
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("ca-bundle.pem");
    let certificate = std::fs::read(path).unwrap();
    let mut certificates = certificate.clone();
    certificates.push(b'\n');
    certificates.extend_from_slice(&certificate);
    std::fs::write(&bundle, certificates).unwrap();
    imported
        .profile
        .elasticsearch_options
        .as_mut()
        .unwrap()
        .ca_cert_path = Some(bundle);
    let bundled =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    bundled.close().await;

    imported.profile.elasticsearch_options = None;
    let error =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Network);

    imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::Basic,
        ca_cert_path: Some(std::path::PathBuf::from(
            std::env::var("LAZYDB_TEST_ELASTICSEARCH_TLS_CERT").unwrap(),
        )),
    });
    imported.profile.host = Some("127.0.0.1".into());
    let error =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Network);
}

#[tokio::test]
#[ignore = "requires an isolated Elasticsearch admin server; uses HTTPS when its fixture URL is set"]
async fn live_api_key_permissions_and_invalidation() {
    use secrecy::ExposeSecret;
    let tls = std::env::var("LAZYDB_TEST_ELASTICSEARCH_TLS_URL").ok();
    let url = tls.clone().unwrap_or_else(|| {
        std::env::var("LAZYDB_TEST_ELASTICSEARCH_URL")
            .expect("set LAZYDB_TEST_ELASTICSEARCH_URL or LAZYDB_TEST_ELASTICSEARCH_TLS_URL")
    });
    let mut imported = import_connection_url(&url, None).unwrap();
    let mut client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));
    if tls.is_some() {
        let path = std::path::PathBuf::from(
            std::env::var("LAZYDB_TEST_ELASTICSEARCH_TLS_CERT")
                .expect("set LAZYDB_TEST_ELASTICSEARCH_TLS_CERT for HTTPS"),
        );
        for certificate in
            reqwest::Certificate::from_pem_bundle(&std::fs::read(&path).unwrap()).unwrap()
        {
            client = client.add_root_certificate(certificate);
        }
        imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
            auth: ElasticsearchAuth::Basic,
            ca_cert_path: Some(path),
        });
    }
    let client = client.build().unwrap();
    let mut endpoint = reqwest::Url::parse(
        &url.replace("elasticsearchs://", "https://")
            .replace("elasticsearch://", "http://"),
    )
    .unwrap();
    endpoint.set_username("").unwrap();
    endpoint.set_password(None).unwrap();
    let admin_request = |method, path: &str| {
        client
            .request(method, endpoint.join(path).unwrap())
            .basic_auth(
                imported.profile.user.as_deref().unwrap(),
                imported
                    .transient_password
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
    };
    let admin =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let index = format!("lazydb-api-key-{suffix}");
    let mut key_id = None;
    let result = AssertUnwindSafe(async {
        admin.execute(&format!("PUT /{index}\n{{}}")).await.unwrap();
        // The key-creation response contains secrets: keep it in the raw test transport,
        // outside query outcomes, application execution events, and query history.
        let created = admin_request(reqwest::Method::POST, "/_security/api_key")
            .body(
                json!({"name":format!("lazydb-test-{suffix}"),"expiration":"1h",
                "role_descriptors":{"reader":{"cluster":["monitor"],
                    "indices":[{"names":[index],"privileges":["read","view_index_metadata"]}]}}})
                .to_string(),
            )
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let created: Value = serde_json::from_slice(&created.bytes().await.unwrap()).unwrap();
        key_id = Some(created["id"].as_str().unwrap().to_owned());
        let encoded = SecretString::from(created["encoded"].as_str().unwrap());
        drop(created);
        let mut profile = imported.profile.clone();
        profile
            .elasticsearch_options
            .get_or_insert_with(Default::default)
            .auth = ElasticsearchAuth::ApiKey;
        // Keep the Basic username to prove API Key authentication overrides it.
        let connection = DatabaseConnection::connect(&profile, Some(&encoded))
            .await
            .unwrap();
        assert!(!format!("{connection:?}").contains(encoded.expose_secret()));
        connection
            .execute(&format!("GET /{index}/_count"))
            .await
            .unwrap();
        let denied = connection
            .execute(&format!("PUT /{index}/_doc/denied\n{{\"n\":99}}"))
            .await
            .unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Permission);
        let invalidated = admin_request(reqwest::Method::DELETE, "/_security/api_key")
            .body(json!({"ids":[key_id.as_ref().unwrap()]}).to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let invalidated: Value =
            serde_json::from_slice(&invalidated.bytes().await.unwrap()).unwrap();
        assert_eq!(
            invalidated["invalidated_api_keys"][0].as_str(),
            key_id.as_deref()
        );
        let denied = connection
            .execute(&format!("GET /{index}/_count"))
            .await
            .unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Authentication);
        connection.close().await;
        let denied = DatabaseConnection::connect(&profile, Some(&encoded))
            .await
            .unwrap_err();
        assert_eq!(denied.category, ErrorCategory::Authentication);
        assert!(!format!("{denied:?}").contains(encoded.expose_secret()));
    })
    .catch_unwind()
    .await;

    let mut cleanup_errors = Vec::new();
    if let Some(id) = key_id {
        match admin_request(reqwest::Method::DELETE, "/_security/api_key")
            .body(json!({"ids":[id]}).to_string())
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            _ => cleanup_errors.push("could not invalidate this test's API key"),
        }
    }
    if let Err(error) = admin.execute(&format!("DELETE /{index}")).await
        && !matches!(
            error.code.as_deref(),
            Some("index_not_found_exception" | "404")
        )
    {
        cleanup_errors.push("could not delete this test's index");
    }
    admin.close().await;
    if let Err(panic) = result {
        if !cleanup_errors.is_empty() {
            eprintln!("resource cleanup errors: {cleanup_errors:?}");
        }
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "resource cleanup errors: {cleanup_errors:?}"
    );
}
