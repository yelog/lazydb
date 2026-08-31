use lazydb::{
    agent::types::{AGENT_API_VERSION, AgentConnection, AgentResponse},
    profile::{DatabaseKind, Environment},
};

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
