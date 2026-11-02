use lazydb::{
    agent::types::{AGENT_API_VERSION, AgentConnection, AgentResponse},
    profile::{DatabaseKind, Environment},
};

#[test]
fn bigquery_job_statistics_survive_agent_projection_and_other_drivers_omit_them() {
    use lazydb::{
        agent::types::QueryOutcomeJson,
        db::query::{BigQueryJobStats, QueryOutcome, QueryStats},
    };
    let mut outcome = QueryOutcome {
        result_sets: vec![],
        stats: QueryStats::new(Default::default(), Default::default(), 0),
    };
    let value = serde_json::to_value(QueryOutcomeJson::from(outcome.clone())).unwrap();
    assert!(value.get("bigquery").is_none());
    outcome.stats.bigquery = Some(Box::new(BigQueryJobStats {
        job_id: "job-123".into(),
        project: "billing-project".into(),
        location: Some("US".into()),
        total_bytes_processed: Some(1024),
        total_bytes_billed: Some(4096),
        slot_millis: Some(12),
        cache_hit: Some(false),
    }));
    let value = serde_json::to_value(QueryOutcomeJson::from(outcome)).unwrap();
    assert_eq!(value["bigquery"]["project"], "billing-project");
    assert_eq!(value["bigquery"]["total_bytes_billed"], 4096);
    assert_eq!(value["bigquery"]["cache_hit"], false);
}

#[test]
fn response_shape_is_versioned_and_connection_projection_has_no_secret_fields() {
    let connection = AgentConnection {
        id: "profile-id".into(),
        name: "dev".into(),
        scope: "global".into(),
        kind: DatabaseKind::Sqlite,
        environment: Environment::Development,
        host: None,
        port: None,
        database: Some("app.db".into()),
        default_schema: Some("main".into()),
        user: None,
        read_only: true,
    };
    let value = serde_json::to_value(AgentResponse {
        api_version: AGENT_API_VERSION,
        ok: true,
        result: connection,
    })
    .unwrap();
    assert_eq!(value["api_version"], 1);
    assert_eq!(value["ok"], true);
    assert!(value.get("password").is_none());
    assert!(value.get("credential_policy").is_none());
}
