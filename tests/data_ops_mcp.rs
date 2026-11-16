use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use lazydb::{
    agent::{
        context::AgentProjectContext,
        mcp::{AgentMcpServer, PlanDataOperationInput},
        policy::WritePolicy,
        service::AgentService,
    },
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobRecord, DataJobSpec, DataJobStatus, DataJobStore,
        DataOperation, NativePlan, NativeToolPaths,
    },
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        profiles::ProfileStore,
        secrets::{SecretStore, SecretStoreError, keyring_ref},
    },
    profile::{
        ConnectionProfile, CredentialPolicy, DatabaseKind, Environment, ProfileAccess,
        import_connection_url,
    },
};
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServiceExt,
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
};
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

struct UnusedSecrets(Arc<AtomicUsize>);

#[async_trait]
impl SecretStore for UnusedSecrets {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }

    async fn get(&self, _: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(SecretStoreError::Unavailable)
    }

    async fn set(&self, _: Uuid, _: &SecretString) -> Result<(), SecretStoreError> {
        panic!("MCP planning must not store credentials")
    }

    async fn delete(&self, _: Uuid) -> Result<(), SecretStoreError> {
        panic!("MCP planning must not delete credentials")
    }
}

fn service(
    root: &Path,
    profiles: Vec<ConnectionProfile>,
    credential_reads: &Arc<AtomicUsize>,
) -> AgentService {
    fs::create_dir_all(root.join(".git")).unwrap();
    AgentService::new(
        AgentProjectContext::resolve(Some(root)).unwrap(),
        profiles,
        CredentialResolver::new(
            Arc::new(UnusedSecrets(credential_reads.clone())),
            LocalCredentialStore::new(root.join("credential.key"), "test"),
        ),
    )
    .with_data_job_store(DataJobStore::new(root.join("data-jobs")))
}

fn postgres_profile() -> ConnectionProfile {
    let mut profile =
        import_connection_url("postgres://test@127.0.0.1:1/source", Some("native-test"))
            .unwrap()
            .profile;
    profile.read_only = false;
    profile.environment = Environment::Development;
    profile.credential_policy = CredentialPolicy::System(keyring_ref(profile.id));
    profile
}

fn sqlite_profile(path: &Path) -> ConnectionProfile {
    let mut profile =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("sqlite"))
            .unwrap()
            .profile;
    profile.read_only = false;
    profile.environment = Environment::Development;
    profile
}

fn endpoint(profile: &ConnectionProfile, database: &str) -> Value {
    json!({"profile_id": profile.id, "database": database})
}

async fn call(
    service: AgentService,
    policy: WritePolicy,
    name: &str,
    input: Value,
) -> Result<Value, String> {
    let server = AgentMcpServer::new(service, policy);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let server_task = tokio::spawn(server.serve(server_io));
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("data-operation-test", "1"),
    );
    let mut client = client_info
        .serve_with_lifecycle((client_read, client_write), ClientLifecycleMode::Initialize)
        .await
        .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(input.as_object().unwrap().clone()),
        )
        .await;
    client.close().await.unwrap();
    drop(client);
    let _server = server_task.await.unwrap().unwrap();
    let result = result.map_err(|error| error.to_string())?;
    let text = result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|content| content.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error.unwrap_or(false) {
        return Err(text);
    }
    serde_json::from_str(&text).map_err(|error| format!("{error}: {text}"))
}

fn record(spec: DataJobSpec) -> DataJobRecord {
    let id = Uuid::new_v4();
    let now = Utc::now();
    DataJobRecord {
        schema_version: 1,
        id,
        plan: DataJobPlan {
            id,
            spec,
            native: NativePlan {
                engine: DatabaseKind::Postgres,
                steps: Vec::new(),
                warnings: Vec::new(),
                tools: Vec::new(),
            },
            profile_fingerprint: "test".into(),
            created_at: now,
        },
        status: DataJobStatus::Succeeded,
        created_at: now,
        updated_at: now,
        message: "Completed test task.".into(),
        bytes: None,
        log: Vec::new(),
        outcome: None,
        error: None,
        target_may_be_partial: false,
    }
}

fn backup_spec(profile: &ConnectionProfile, artifact: &Path) -> DataJobSpec {
    DataJobSpec {
        operation: DataOperation::Backup,
        source: Some(DataEndpoint {
            profile_id: profile.id,
            database: "source".into(),
        }),
        destination: None,
        artifact: artifact.to_owned(),
        tools: NativeToolPaths::default(),
    }
}

#[test]
fn data_tools_are_read_only_and_input_cannot_choose_executables() {
    for tool in [
        AgentMcpServer::plan_data_operation_tool_attr(),
        AgentMcpServer::list_data_tasks_tool_attr(),
        AgentMcpServer::get_data_task_tool_attr(),
    ] {
        let annotations = tool.annotations.unwrap();
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
    }
    let tool = AgentMcpServer::plan_data_operation_tool_attr();
    assert!(
        !tool.input_schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("tools")
    );
    assert!(
        tool.description
            .as_deref()
            .unwrap()
            .contains("does not create")
    );
    let profile = postgres_profile();
    let input = json!({
        "operation": "backup",
        "source": endpoint(&profile, "source"),
        "artifact": "snapshot.dump"
    });
    let spec = DataJobSpec::try_from(
        serde_json::from_value::<PlanDataOperationInput>(input.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(spec.tools, NativeToolPaths::default());
    for field in ["tools", "dump", "restore", "executable"] {
        let mut overridden = input.clone();
        overridden[field] = json!({"dump": "/tmp/arbitrary-program"});
        assert!(serde_json::from_value::<PlanDataOperationInput>(overridden).is_err());
    }
    let mut overridden = input;
    overridden["source"]["tools"] = json!({"dump": "/tmp/arbitrary-program"});
    assert!(serde_json::from_value::<PlanDataOperationInput>(overridden).is_err());
}

#[test]
fn data_service_uses_only_visible_profiles_and_the_injected_store() {
    let temp = tempfile::tempdir().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let global = postgres_profile();
    let mut current = postgres_profile();
    current.access = ProfileAccess::Projects {
        roots: vec![temp.path().canonicalize().unwrap()],
    };
    let mut hidden = postgres_profile();
    hidden.access = ProfileAccess::Projects {
        roots: vec![temp.path().join("another-project")],
    };
    let service = service(
        temp.path(),
        vec![global.clone(), current.clone(), hidden.clone()],
        &reads,
    );
    let data = service.data_job_service().unwrap();
    assert_eq!(
        data.profiles()
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>(),
        vec![global.id, current.id]
    );
    assert_eq!(data.store().directory(), temp.path().join("data-jobs"));
    assert!(!data.can_view(&backup_spec(&hidden, Path::new("snapshot.dump"))));
    assert!(data.can_view(&backup_spec(&global, Path::new("snapshot.dump"))));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!data.store().directory().exists());
}

#[test]
fn explicit_config_selects_sibling_data_history_without_creating_it() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    let config = temp.path().join("custom-config").join("connections.toml");
    ProfileStore::new(config.clone())
        .save(Vec::<ConnectionProfile>::new())
        .unwrap();
    let service = AgentService::load(Some(&project), Some(config.clone())).unwrap();
    let data = service.data_job_service().unwrap();
    let expected = config
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join("data-jobs");
    assert_eq!(data.store().directory(), expected);
    assert!(!expected.exists());
}

#[tokio::test]
async fn destination_policy_is_checked_before_credentials_or_native_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    fs::write(temp.path().join("input.dump"), b"not a native archive").unwrap();
    for (policy, read_only, environment, expected) in [
        (
            WritePolicy::Deny,
            false,
            Environment::Development,
            "write policy is deny",
        ),
        (
            WritePolicy::All,
            true,
            Environment::Development,
            "configured as read-only",
        ),
        (
            WritePolicy::NonProduction,
            false,
            Environment::Production,
            "production writes require",
        ),
    ] {
        let mut profile = postgres_profile();
        profile.read_only = read_only;
        profile.environment = environment;
        for operation in ["restore", "copy"] {
            let input = json!({
                "operation": operation,
                "source": (operation == "copy").then(|| endpoint(&profile, "source")),
                "destination": endpoint(&profile, "new_database"),
                "artifact": if operation == "restore" { "input.dump" } else { "output.dump" }
            });
            let error = call(
                service(temp.path(), vec![profile.clone()], &reads),
                policy,
                "plan_data_operation",
                input,
            )
            .await
            .unwrap_err();
            assert!(error.contains(expected), "{operation}: {error}");
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
    }
    assert!(!temp.path().join("output.dump").exists());
    assert!(!temp.path().join("data-jobs").exists());
}

#[tokio::test]
async fn hidden_and_unknown_connections_are_rejected_before_paths_or_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let visible = postgres_profile();
    let mut hidden = postgres_profile();
    hidden.access = ProfileAccess::Projects {
        roots: vec![temp.path().join("another-project")],
    };
    let unknown = postgres_profile();
    for unavailable in [&hidden, &unknown] {
        for input in [
            json!({
                "operation": "backup",
                "source": endpoint(unavailable, "private_source"),
                "artifact": "missing-directory/archive.dump"
            }),
            json!({
                "operation": "restore",
                "destination": endpoint(unavailable, "private_destination"),
                "artifact": "missing-directory/archive.dump"
            }),
            json!({
                "operation": "copy",
                "source": endpoint(&visible, "source"),
                "destination": endpoint(unavailable, "private_destination"),
                "artifact": "missing-directory/archive.dump"
            }),
        ] {
            let error = call(
                service(temp.path(), vec![visible.clone(), hidden.clone()], &reads),
                WritePolicy::All,
                "plan_data_operation",
                input,
            )
            .await
            .unwrap_err();
            assert!(error.contains("unavailable connection"), "{error}");
            assert!(!error.contains("private_"));
            assert!(!error.contains(&unavailable.id.to_string()));
        }
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!temp.path().join("data-jobs").exists());
}

#[tokio::test]
async fn artifact_paths_must_stay_in_the_project_before_native_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let profile = postgres_profile();
    let outside_input = temp.path().join("input.dump");
    let outside_output = temp.path().join("output.dump");
    fs::write(&outside_input, b"private archive").unwrap();
    for operation in ["backup", "restore", "copy"] {
        for relative in [true, false] {
            let path = if operation == "restore" {
                &outside_input
            } else {
                &outside_output
            };
            let artifact = if relative {
                format!("../{}", path.file_name().unwrap().to_str().unwrap())
            } else {
                path.to_string_lossy().into_owned()
            };
            let error = call(
                service(&project, vec![profile.clone()], &reads),
                WritePolicy::All,
                "plan_data_operation",
                json!({
                    "operation": operation,
                    "source": (operation != "restore").then(|| endpoint(&profile, "source")),
                    "destination": (operation != "backup").then(|| endpoint(&profile, "new_database")),
                    "artifact": artifact
                }),
            )
            .await
            .unwrap_err();
            assert!(
                error.contains("outside the project root"),
                "{operation}: {error}"
            );
        }
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(outside_input).unwrap(), b"private archive");
    assert!(!outside_output.exists());
    assert!(!project.join("data-jobs").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn artifact_symlinks_cannot_escape_the_project() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("input.dump"), b"private archive").unwrap();
    symlink(&outside, project.join("linked-directory")).unwrap();
    symlink(
        outside.join("input.dump"),
        project.join("linked-input.dump"),
    )
    .unwrap();
    symlink(outside.join("absent.dump"), project.join("dangling.dump")).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let profile = postgres_profile();
    for (operation, artifact, expected) in [
        (
            "backup",
            "linked-directory/output.dump",
            "outside the project root",
        ),
        ("restore", "linked-input.dump", "outside the project root"),
        ("backup", "linked-input.dump", "outside the project root"),
        ("backup", "dangling.dump", "Cannot resolve"),
    ] {
        let error = call(
            service(&project, vec![profile.clone()], &reads),
            WritePolicy::All,
            "plan_data_operation",
            json!({
                "operation": operation,
                "source": (operation == "backup").then(|| endpoint(&profile, "source")),
                "destination": (operation == "restore").then(|| endpoint(&profile, "new_database")),
                "artifact": artifact
            }),
        )
        .await
        .unwrap_err();
        assert!(error.contains(expected), "{artifact}: {error}");
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!outside.join("output.dump").exists());
    assert!(!outside.join("absent.dump").exists());
}

#[tokio::test]
async fn sqlite_source_and_destination_paths_are_scoped_before_opening_databases() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("source.db"), b"not a SQLite database").unwrap();
    fs::write(temp.path().join("private.db"), b"private database").unwrap();
    let profile = sqlite_profile(&project.join("source.db"));
    let reads = Arc::new(AtomicUsize::new(0));
    for input in [
        json!({
            "operation": "backup",
            "source": endpoint(&profile, "../private.db"),
            "artifact": "backup.db"
        }),
        json!({
            "operation": "restore",
            "destination": endpoint(&profile, "../new.db"),
            "artifact": "source.db"
        }),
        json!({
            "operation": "copy",
            "source": endpoint(&profile, "source.db"),
            "destination": endpoint(&profile, "../new.db"),
            "artifact": "backup.db"
        }),
    ] {
        let error = call(
            service(&project, vec![profile.clone()], &reads),
            WritePolicy::All,
            "plan_data_operation",
            input,
        )
        .await
        .unwrap_err();
        assert!(error.contains("outside the project root"), "{error}");
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read(temp.path().join("private.db")).unwrap(),
        b"private database"
    );
    assert!(!temp.path().join("new.db").exists());
    assert!(!project.join("backup.db").exists());
    assert!(!project.join("data-jobs").exists());
}

#[tokio::test]
async fn sqlite_plans_resolve_relative_paths_from_project_and_create_no_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let source_path = project.join("source.db");
    let mut source = sqlite_profile(&source_path);
    let setup = lazydb::db::DatabaseConnection::connect(&source, None)
        .await
        .unwrap();
    setup
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    setup
        .execute("INSERT INTO records VALUES (1, 'original')")
        .await
        .unwrap();
    setup.close().await;
    let source_bytes = fs::read(&source_path).unwrap();
    let destination = sqlite_profile(&project.join("connection-default.db"));
    source.sqlite_path = Some("source.db".into());
    source.read_only = true;
    let reads = Arc::new(AtomicUsize::new(0));
    for (operation, policy) in [
        ("backup", WritePolicy::Deny),
        ("restore", WritePolicy::NonProduction),
        ("copy", WritePolicy::NonProduction),
    ] {
        let artifact = if operation == "restore" {
            "source.db"
        } else {
            "snapshot.db"
        };
        let result = call(
            service(&project, vec![source.clone(), destination.clone()], &reads),
            policy,
            "plan_data_operation",
            json!({
                "operation": operation,
                "source": (operation != "restore").then(|| endpoint(&source, "source.db")),
                "destination": (operation != "backup").then(|| endpoint(&destination, "destination.db")),
                "artifact": artifact
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["review_only"], true);
        assert_eq!(result["execution_available"], false);
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("no output artifact")
        );
        assert_eq!(result["plan"]["native"]["engine"], "sqlite");
        assert_eq!(
            result["plan"]["spec"]["artifact"],
            project
                .canonicalize()
                .unwrap()
                .join(artifact)
                .to_str()
                .unwrap()
        );
        if operation != "restore" {
            assert_eq!(
                result["plan"]["spec"]["source"]["database"],
                source_path.canonicalize().unwrap().to_str().unwrap()
            );
        }
        if operation != "backup" {
            assert_eq!(
                result["plan"]["spec"]["destination"]["database"],
                project
                    .canonicalize()
                    .unwrap()
                    .join("destination.db")
                    .to_str()
                    .unwrap()
            );
        }
        assert!(!project.join("snapshot.db").exists());
        assert!(!project.join("snapshot.db.lazydb.json").exists());
        assert!(!project.join("destination.db").exists());
        assert!(!project.join("connection-default.db").exists());
        assert!(!project.join("data-jobs").exists());
        assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn history_filters_profiles_and_paths_and_hidden_lookups_are_generic() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let visible = postgres_profile();
    let mut hidden = postgres_profile();
    hidden.access = ProfileAccess::Projects {
        roots: vec![temp.path().join("another-project")],
    };
    let sqlite = sqlite_profile(&project.join("source.db"));
    let store = DataJobStore::new(project.join("data-jobs"));
    let allowed = record(backup_spec(&visible, Path::new("allowed.dump")));
    store.create(&allowed).unwrap();
    let mut denied = Vec::new();
    denied.push(record(backup_spec(&hidden, Path::new("hidden.dump"))));
    let mut hidden_destination = backup_spec(&visible, Path::new("hidden-copy.dump"));
    hidden_destination.operation = DataOperation::Copy;
    hidden_destination.destination = Some(DataEndpoint {
        profile_id: hidden.id,
        database: "private_destination".into(),
    });
    denied.push(record(hidden_destination));
    denied.push(record(backup_spec(
        &visible,
        &temp.path().join("outside.dump"),
    )));
    let mut outside_sqlite = backup_spec(&sqlite, Path::new("sqlite.dump"));
    outside_sqlite.source.as_mut().unwrap().database = "../outside.db".into();
    denied.push(record(outside_sqlite));
    for task in &mut denied {
        task.message = "private task detail".into();
        store.create(task).unwrap();
    }
    let profiles = vec![visible, hidden, sqlite];
    let listed = call(
        service(&project, profiles.clone(), &reads),
        WritePolicy::Deny,
        "list_data_tasks",
        json!({"limit": 1}),
    )
    .await
    .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], allowed.id.to_string());
    assert!(!listed.to_string().contains("private"));
    let retrieved = call(
        service(&project, profiles.clone(), &reads),
        WritePolicy::Deny,
        "get_data_task",
        json!({"id": allowed.id}),
    )
    .await
    .unwrap();
    assert_eq!(retrieved["id"], allowed.id.to_string());
    for id in denied
        .iter()
        .map(|task| task.id.to_string())
        .chain([Uuid::new_v4().to_string(), "not-a-task-id".into()])
    {
        let error = call(
            service(&project, profiles.clone(), &reads),
            WritePolicy::Deny,
            "get_data_task",
            json!({"id": id}),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "The data task is unavailable.");
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn data_history_responses_obey_the_server_result_size_limit() {
    let temp = tempfile::tempdir().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let profile = postgres_profile();
    let store = DataJobStore::new(temp.path().join("data-jobs"));
    let task = record(backup_spec(&profile, Path::new("backup.dump")));
    store.create(&task).unwrap();
    for (name, input) in [
        ("list_data_tasks", json!({})),
        ("get_data_task", json!({"id": task.id})),
    ] {
        let error = call(
            service(temp.path(), vec![profile.clone()], &reads).with_limits(500, 32),
            WritePolicy::Deny,
            name,
            input,
        )
        .await
        .unwrap_err();
        assert!(error.contains("result-size limit"), "{error}");
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}
