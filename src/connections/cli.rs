use std::{
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    agent::{context::AgentProjectContext, selection::select_profile},
    cli::{ConnectionScope, ConnectionsCommand},
    config::ConnectionAccessDefault,
    db::DatabaseConnection,
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        paths::AppPaths,
        profiles::{ProfileMutationError, ProfileStore},
        secrets::{NativeSecretStore, SecretStore, keyring_ref, profile_id_from_ref},
        settings::AppSettings,
    },
    profile::{
        ConnectionProfile, CredentialPolicy, ProfileAccess, ProfileCollection,
        import_connection_url,
    },
    project::ProjectContext,
};

const MAX_PASSWORD_BYTES: usize = 16 * 1024 - 64;

impl From<ConnectionAccessDefault> for ConnectionScope {
    fn from(value: ConnectionAccessDefault) -> Self {
        match value {
            ConnectionAccessDefault::Global => Self::Global,
            ConnectionAccessDefault::Project => Self::Project,
        }
    }
}

impl ConnectionsCommand {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "connections.add",
            Self::List { .. } => "connections.list",
            Self::Show { .. } => "connections.show",
            Self::Test { .. } => "connections.test",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionCliError {
    pub code: &'static str,
    pub message: String,
    pub exit_code: i32,
}

impl std::fmt::Display for ConnectionCliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConnectionCliError {}

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    schema_version: u8,
    ok: bool,
    command: &'static str,
    data: Option<T>,
    error: Option<ErrorBody>,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ConnectionSummary {
    id: Uuid,
    name: String,
    kind: crate::profile::DatabaseKind,
    scope: &'static str,
    projects: Vec<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    database: Option<String>,
    default_schema: Option<String>,
    user: Option<String>,
    read_only: bool,
    credential_storage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<bool>,
}

#[derive(Serialize)]
struct TestSummary {
    connection: ConnectionSummary,
    server: crate::db::ServerInfo,
}

#[derive(Serialize)]
struct UnavailableConnectionSummary {
    id: Option<Uuid>,
    name: String,
    kind: Option<String>,
    reason: &'static str,
}

#[derive(Serialize)]
struct ListSummary {
    connections: Vec<ConnectionSummary>,
    unavailable: Vec<UnavailableConnectionSummary>,
}

pub async fn run(
    config: Option<PathBuf>,
    url: Option<String>,
    profile_selector: Option<String>,
    cli_read_only: bool,
    command: ConnectionsCommand,
    json: bool,
) -> Result<String, ConnectionCliError> {
    if profile_selector.is_some() {
        return Err(error(
            "invalid_arguments",
            "--profile is not valid with the connections command.",
            2,
        ));
    }
    if !matches!(command, ConnectionsCommand::Add { .. }) && (url.is_some() || cli_read_only) {
        return Err(error(
            "invalid_arguments",
            "--url and --read-only are only valid with connections add.",
            2,
        ));
    }
    match command {
        ConnectionsCommand::Add {
            name,
            scope,
            project,
            password_env,
            password_stdin,
            upsert,
            read_write,
        } => {
            if cli_read_only && read_write {
                return Err(error(
                    "invalid_arguments",
                    "--read-only and --read-write cannot be used together.",
                    2,
                ));
            }
            let Some(url) = url.as_deref() else {
                return Err(error(
                    "invalid_arguments",
                    "connections add requires --url.",
                    2,
                ));
            };
            if upsert && scope.is_none() {
                return Err(error(
                    "invalid_arguments",
                    "--upsert requires an explicit --scope.",
                    2,
                ));
            }
            let paths = app_paths()?;
            let scope = match scope {
                Some(scope) => scope,
                None => AppSettings::load(paths.settings_file())
                    .map_err(|_| {
                        error(
                            "invalid_configuration",
                            "Unable to read connection defaults.",
                            2,
                        )
                    })?
                    .connections
                    .default_access
                    .into(),
            };
            if upsert && scope == ConnectionScope::Global && project.is_none() {
                // Global upserts still need a stable project context to validate
                // visibility and report the same canonical project on all hosts.
            }
            let context =
                ProjectContext::resolve_from(project.as_deref().unwrap_or(Path::new(".")))
                    .map_err(|_| {
                        error("invalid_project", "Unable to resolve the project path.", 2)
                    })?;
            let mut imported = import_connection_url(url, Some(&name)).map_err(|_| {
                error(
                    "invalid_profile",
                    "The connection URL is invalid or unsupported.",
                    2,
                )
            })?;
            let normalized_name = name.trim();
            if normalized_name.is_empty() {
                return Err(error(
                    "invalid_profile",
                    "Connection name cannot be empty.",
                    2,
                ));
            }
            imported.profile.name = normalized_name.to_owned();
            if let Some(path) = imported.profile.sqlite_path.as_mut()
                && path.is_relative()
            {
                *path = std::env::current_dir()
                    .map_err(|_| error("invalid_profile", "Unable to resolve the SQLite path.", 2))?
                    .join(&*path);
                imported.profile.database = Some(path.to_string_lossy().into_owned());
                imported.profile.catalog_scope = crate::profile::CatalogScope::for_profile(
                    imported.profile.kind,
                    &imported.profile.database.clone().unwrap_or_default(),
                    imported.profile.default_schema.as_deref(),
                );
            }
            let explicit_password = read_password(
                password_env.as_deref(),
                password_stdin,
                imported.transient_password.is_some(),
            )?;
            let password = explicit_password.or(imported.transient_password.take());
            let url_read_only_explicit = imported.read_only_explicit;
            imported.profile.read_only = if read_write {
                false
            } else {
                imported.profile.read_only || cli_read_only
            };
            imported.profile.access = match scope {
                ConnectionScope::Global => ProfileAccess::Global,
                ConnectionScope::Project => ProfileAccess::Projects {
                    roots: vec![context.root.clone()],
                },
            };

            let profile_path = config.unwrap_or_else(|| paths.profiles_file());
            let store = ProfileStore::new(profile_path)
                .with_credential_key_path(paths.credential_key_file());
            let local_store = LocalCredentialStore::new(store.credential_key_path(), "lazydb")
                .with_fallback_key_path(
                    store
                        .path()
                        .parent()
                        .map(|parent| parent.join("credential.key"))
                        .unwrap_or_else(|| PathBuf::from("credential.key")),
                );
            let native_store = std::sync::Arc::new(NativeSecretStore);
            let rollback_state =
                std::sync::Arc::new(std::sync::Mutex::new(None::<(Uuid, Option<SecretString>)>));
            let rollback_store = native_store.clone();
            let rollback_snapshot = rollback_state.clone();
            let rollback = move || {
                let store = rollback_store.clone();
                let state = rollback_snapshot.clone();
                async move {
                    let pending = {
                        state
                            .lock()
                            .map_err(|_| "credential rollback state is unavailable".to_owned())?
                            .take()
                    };
                    let Some((profile_id, previous)) = pending else {
                        return Ok(());
                    };
                    match previous {
                        Some(password) => store.set(profile_id, &password).await.map_err(|_| {
                            "unable to restore the previous system credential".to_owned()
                        }),
                        None => store.delete(profile_id).await.map_err(|_| {
                            "unable to remove the partially written system credential".to_owned()
                        }),
                    }
                }
            };
            let original_scope = scope;
            let original_root = context.root;
            let explicit_read_only = cli_read_only || read_write || url_read_only_explicit;
            let profile = imported.profile;
            let password = password;
            let operation_native_store = native_store.clone();
            let operation_rollback_state = rollback_state.clone();
            let result = store
                .mutate_async_with_rollback(move |mut collection| async move {
                    let canonical_name = profile.name.to_lowercase();
                    let matches = collection
                        .profiles
                        .iter()
                        .enumerate()
                        .filter(|(_, existing)| {
                            existing.name.trim().to_lowercase() == canonical_name
                                && match original_scope {
                                    ConnectionScope::Global => {
                                        matches!(existing.access, ProfileAccess::Global)
                                    }
                                    ConnectionScope::Project => existing
                                        .access
                                        .contains_project(&original_root),
                                }
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    if matches.len() > 1 {
                        return Err("connection_ambiguous: multiple profiles match the requested scope".to_owned());
                    }
                    if matches.is_empty()
                        && collection.profiles.iter().any(|existing| {
                            existing.name.trim().eq_ignore_ascii_case(&profile.name)
                        })
                    {
                        return Err("connection_name_conflict: a profile with this name exists in another scope".to_owned());
                    }
                    let index = matches.first().copied();
                    if index.is_some() && !upsert {
                        return Err("connection_name_conflict: a profile with this name already exists".to_owned());
                    }
                    let mut saved = profile;
                    let previous = index.map(|index| collection.profiles[index].clone());
                    let mut credential_changed = false;
                    if let Some(previous) = &previous {
                        saved.id = previous.id;
                        saved.group_id = previous.group_id;
                        saved.environment = previous.environment;
                        saved.access = previous.access.clone();
                        saved.credential_policy = previous.credential_policy.clone();
                        if !explicit_read_only {
                            saved.read_only = previous.read_only;
                        }
                        if previous.catalog_scope != crate::profile::CatalogScope::for_profile(
                            previous.kind,
                            previous.database.as_deref().unwrap_or_default(),
                            previous.default_schema.as_deref(),
                        ) && previous.kind == saved.kind
                        {
                            saved.catalog_scope = previous.catalog_scope.clone();
                        }
                        saved
                            .catalog_scope
                            .validate(
                                saved.database.as_deref().unwrap_or_default(),
                                saved.default_schema.as_deref(),
                            )
                            .map_err(|_| "catalog_scope_conflict: saved catalog scope is incompatible with the new URL".to_owned())?;
                    }
                    if let Some(password) = password {
                        if password.expose_secret().len() > MAX_PASSWORD_BYTES {
                            return Err("credential_input_invalid: password is too long".to_owned());
                        }
                        let replace_system_credential = match &saved.credential_policy {
                            CredentialPolicy::System(reference)
                            | CredentialPolicy::Keyring(reference) => {
                                let referenced = profile_id_from_ref(reference)
                                    .map_err(|_| "credential_failure: invalid system credential reference".to_owned())?;
                                if referenced != saved.id {
                                    return Err("credential_failure: system credential reference does not match the profile".to_owned());
                                }
                                true
                            }
                            _ => false,
                        };
                        if replace_system_credential {
                            let previous = operation_native_store
                                .get(saved.id)
                                .await
                                .map_err(|_| "credential_failure: unable to read the previous system credential".to_owned())?;
                            credential_changed = previous.as_ref().is_none_or(|previous| {
                                previous.expose_secret() != password.expose_secret()
                            });
                            if credential_changed {
                                *operation_rollback_state
                                    .lock()
                                    .map_err(|_| "credential rollback state is unavailable".to_owned())? =
                                    Some((saved.id, previous));
                                operation_native_store
                                    .set(saved.id, &password)
                                    .await
                                    .map_err(|_| "credential_failure: unable to write the new system credential".to_owned())?;
                            }
                            saved.credential_policy = CredentialPolicy::System(keyring_ref(saved.id));
                        }
                        let already_stored = match &saved.credential_policy {
                            CredentialPolicy::LocalEncrypted(credential) => local_store
                                .decrypt(saved.id, credential)
                                .is_ok_and(|previous| previous.expose_secret() == password.expose_secret()),
                            _ => false,
                        };
                        if !replace_system_credential && !already_stored {
                            credential_changed = true;
                            let encrypted = local_store
                                .encrypt(saved.id, &password)
                                .map_err(|_| "persistence_failure: unable to encrypt the password".to_owned())?;
                            saved.credential_policy = CredentialPolicy::LocalEncrypted(encrypted);
                        }
                    } else if previous.is_none() {
                        saved.credential_policy = CredentialPolicy::None;
                    }
                    let changed = credential_changed || previous.as_ref() != Some(&saved);
                    if let Some(index) = index {
                        collection.profiles[index] = saved.clone();
                    } else {
                        collection.profiles.push(saved.clone());
                    }
                    let summary = summarize(&saved, None);
                    Ok((collection, (summary, changed), changed))
                }, rollback)
                .await;
            let result = result.map_err(map_mutation_error)?;
            let (mut summary, changed) = result;
            summary.changed = Some(changed);
            Ok(render("connections.add", summary, json))
        }
        ConnectionsCommand::List { project, all } => {
            let (store, context) = read_context(config.as_deref(), project.as_deref())?;
            let report = store.load_report().map_err(|_| {
                error(
                    "persistence_failure",
                    "Unable to read connection profiles.",
                    5,
                )
            })?;
            let profiles = if all {
                report.collection.profiles
            } else {
                context
                    .visible_profiles(&report.collection.profiles)
                    .into_iter()
                    .map(|entry| entry.profile.clone())
                    .collect()
            };
            let summaries = profiles
                .iter()
                .map(|profile| summarize(profile, None))
                .collect::<Vec<_>>();
            let unavailable = if all {
                report
                    .unavailable
                    .into_iter()
                    .map(|profile| UnavailableConnectionSummary {
                        id: profile.id,
                        name: profile.name,
                        kind: profile.kind,
                        reason: match profile.reason {
                            crate::profile_compatibility::ProfileUnavailableReason::UnsupportedKind => "unsupported_kind",
                            crate::profile_compatibility::ProfileUnavailableReason::UnsupportedConfiguration => "unsupported_configuration",
                            crate::profile_compatibility::ProfileUnavailableReason::InvalidConfiguration => "invalid_configuration",
                        },
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Ok(render(
                "connections.list",
                ListSummary {
                    connections: summaries,
                    unavailable,
                },
                json,
            ))
        }
        ConnectionsCommand::Show {
            selector,
            project,
            all,
        } => {
            let (store, context) = read_context(config.as_deref(), project.as_deref())?;
            let report = store.load_report().map_err(|_| {
                error(
                    "persistence_failure",
                    "Unable to read connection profiles.",
                    5,
                )
            })?;
            let profile =
                select_profile_from_collection(&report.collection, &context, &selector, all)?;
            Ok(render("connections.show", summarize(profile, None), json))
        }
        ConnectionsCommand::Test {
            selector,
            project,
            timeout,
        } => {
            let (store, context) = read_context(config.as_deref(), project.as_deref())?;
            let report = store.load_report().map_err(|_| {
                error(
                    "persistence_failure",
                    "Unable to read connection profiles.",
                    5,
                )
            })?;
            let profile =
                select_profile_from_collection(&report.collection, &context, &selector, false)?;
            if profile.kind == crate::profile::DatabaseKind::Sqlite
                && let Some(path) = profile.sqlite_path.as_ref()
                && !path.exists()
            {
                return Err(error(
                    "connection_failed",
                    "The SQLite database file does not exist; test does not create it.",
                    6,
                ));
            }
            let deadline = Instant::now() + Duration::from_secs(timeout);
            let remaining = || deadline.saturating_duration_since(Instant::now());
            let resolver = CredentialResolver::new(
                std::sync::Arc::new(NativeSecretStore),
                LocalCredentialStore::new(store.credential_key_path(), "lazydb")
                    .with_fallback_key_path(
                        store
                            .path()
                            .parent()
                            .map(|parent| parent.join("credential.key"))
                            .unwrap_or_else(|| PathBuf::from("credential.key")),
                    ),
            );
            let password = tokio::time::timeout(remaining(), resolver.resolve_headless(profile))
                .await
                .map_err(|_| error("timeout", "Connection test timed out.", 6))?
                .map_err(|_| {
                    error(
                        "credential_failure",
                        "The saved credential is unavailable without interaction.",
                        4,
                    )
                })?;
            let connection = tokio::time::timeout(
                remaining(),
                DatabaseConnection::connect(profile, password.as_ref()),
            )
            .await
            .map_err(|_| error("timeout", "Connection test timed out.", 6))?
            .map_err(|_| error("connection_failed", "Unable to connect to the database.", 6))?;
            let probe = tokio::time::timeout(remaining(), connection.probe()).await;
            let server = match probe {
                Ok(Ok(server)) => server,
                Ok(Err(_)) => {
                    close_test_connection(connection).await;
                    return Err(error("connection_failed", "Database probe failed.", 6));
                }
                Err(_) => {
                    close_test_connection(connection).await;
                    return Err(error("timeout", "Connection test timed out.", 6));
                }
            };
            close_test_connection(connection).await;
            Ok(render(
                "connections.test",
                TestSummary {
                    connection: summarize(profile, None),
                    server,
                },
                json,
            ))
        }
    }
}

async fn close_test_connection(connection: DatabaseConnection) {
    let _ = tokio::time::timeout(Duration::from_secs(2), connection.close()).await;
}

fn app_paths() -> Result<AppPaths, ConnectionCliError> {
    AppPaths::discover().map_err(|_| {
        error(
            "persistence_failure",
            "Unable to locate LazyDB configuration.",
            5,
        )
    })
}

fn read_context(
    config: Option<&Path>,
    project: Option<&Path>,
) -> Result<(ProfileStore, AgentProjectContext), ConnectionCliError> {
    let paths = app_paths()?;
    let project = AgentProjectContext::resolve(project)
        .map_err(|_| error("invalid_project", "Unable to resolve the project path.", 2))?;
    let store = ProfileStore::new(
        config
            .map(Path::to_owned)
            .unwrap_or_else(|| paths.profiles_file()),
    )
    .with_credential_key_path(paths.credential_key_file());
    Ok((store, project))
}

fn select_profile_from_collection<'a>(
    collection: &'a ProfileCollection,
    context: &AgentProjectContext,
    selector: &str,
    all: bool,
) -> Result<&'a ConnectionProfile, ConnectionCliError> {
    let selected = if all {
        let matches = collection
            .profiles
            .iter()
            .filter(|profile| {
                Uuid::parse_str(selector).is_ok_and(|id| id == profile.id)
                    || profile.name == selector
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [profile] => Some(*profile),
            [] => None,
            _ => {
                return Err(error(
                    "connection_ambiguous",
                    "Connection name is ambiguous.",
                    3,
                ));
            }
        }
    } else {
        let visible = context.visible_profiles(&collection.profiles);
        return select_profile(&visible, Some(selector))
            .map(|selected| selected.profile)
            .map_err(|selection_error| match selection_error.code {
                crate::agent::selection::AgentErrorCode::ConnectionAmbiguous => {
                    error("connection_ambiguous", "Connection name is ambiguous.", 3)
                }
                _ => error("connection_not_found", "Connection not found.", 3),
            });
    };
    selected.ok_or_else(|| error("connection_not_found", "Connection not found.", 3))
}

fn summarize(profile: &ConnectionProfile, changed: Option<bool>) -> ConnectionSummary {
    let (scope, projects) = match &profile.access {
        ProfileAccess::Global => ("global", Vec::new()),
        ProfileAccess::Projects { roots } => ("project", roots.clone()),
    };
    let credential_storage = match profile.credential_policy {
        CredentialPolicy::None => "none",
        CredentialPolicy::Prompt => "prompt",
        CredentialPolicy::LocalEncrypted(_) => "local_encrypted",
        CredentialPolicy::System(_) | CredentialPolicy::Keyring(_) => "system",
    };
    ConnectionSummary {
        id: profile.id,
        name: profile.name.clone(),
        kind: profile.kind,
        scope,
        projects,
        host: profile.host.clone(),
        port: profile.port,
        database: profile.database.clone(),
        default_schema: profile.default_schema.clone(),
        user: profile.user.clone(),
        read_only: profile.read_only,
        credential_storage,
        changed,
    }
}

fn render<T: Serialize>(command: &'static str, data: T, json: bool) -> String {
    if json {
        serde_json::to_string(&Envelope {
            schema_version: 1,
            ok: true,
            command,
            data: Some(data),
            error: None,
        })
        .unwrap_or_else(|_| "{\"schema_version\":1,\"ok\":false}".to_owned())
    } else {
        let value = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
        let _ = command;
        render_text(&value)
    }
}

fn render_text(value: &serde_json::Value) -> String {
    if let Some(connections) = value
        .get("connections")
        .and_then(serde_json::Value::as_array)
    {
        if connections.is_empty() {
            return "No connections configured.".to_owned();
        }
        let mut lines = connections
            .iter()
            .map(render_connection_line)
            .collect::<Vec<_>>();
        if let Some(unavailable) = value
            .get("unavailable")
            .and_then(serde_json::Value::as_array)
            && !unavailable.is_empty()
        {
            lines.push(format!("{} unavailable connection(s)", unavailable.len()));
        }
        return lines.join("\n");
    }
    if let Some(connection) = value.get("connection") {
        let mut line = render_connection_line(connection);
        if let Some(server) = value.get("server") {
            let version = server
                .get("version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown version");
            line.push_str(&format!("; connected ({version})"));
        }
        return line;
    }
    render_connection_line(value)
}

fn render_connection_line(value: &serde_json::Value) -> String {
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("connection");
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("database");
    let scope = value
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let changed = value
        .get("changed")
        .and_then(serde_json::Value::as_bool)
        .map(|changed| format!(" changed={changed}"))
        .unwrap_or_default();
    format!("{name} ({kind}, {scope}) id={id}{changed}")
}

pub fn render_error(command: &'static str, error: &ConnectionCliError, json: bool) -> String {
    if json {
        serde_json::to_string(&Envelope::<serde_json::Value> {
            schema_version: 1,
            ok: false,
            command,
            data: None,
            error: Some(ErrorBody {
                code: error.code,
                message: error.message.clone(),
            }),
        })
        .unwrap_or_else(|_| "{\"schema_version\":1,\"ok\":false}".to_owned())
    } else {
        error.to_string()
    }
}

fn error(code: &'static str, message: &str, exit_code: i32) -> ConnectionCliError {
    ConnectionCliError {
        code,
        message: message.to_owned(),
        exit_code,
    }
}

fn map_mutation_error(mutation_error: ProfileMutationError) -> ConnectionCliError {
    match mutation_error {
        ProfileMutationError::Rejected(message) => {
            if message.starts_with("connection_ambiguous:") {
                error(
                    "connection_ambiguous",
                    "Multiple connections match this name and scope.",
                    3,
                )
            } else if message.starts_with("connection_name_conflict:") {
                error(
                    "connection_name_conflict",
                    "A connection with this name already exists.",
                    3,
                )
            } else if message.starts_with("credential_input_invalid:") {
                error(
                    "credential_input_invalid",
                    "Password input is invalid or too long.",
                    4,
                )
            } else if message.starts_with("persistence_failure:") {
                error(
                    "persistence_failure",
                    "Unable to encrypt or save the connection.",
                    5,
                )
            } else if message.starts_with("credential_failure:") {
                error(
                    "credential_failure",
                    "The existing system credential cannot be safely replaced by this command.",
                    4,
                )
            } else if message.starts_with("catalog_scope_conflict:") {
                error(
                    "catalog_scope_conflict",
                    "The saved catalog scope is incompatible with the new connection URL.",
                    2,
                )
            } else if message.starts_with("credential_rollback_failed:") {
                error(
                    "credential_rollback_failed",
                    "The profile could not be saved and the previous credential could not be restored.",
                    5,
                )
            } else {
                error("invalid_profile", "Connection profile is invalid.", 2)
            }
        }
        ProfileMutationError::Lock(
            crate::persistence::profile_transaction::ProfileLockError::Busy,
        ) => error(
            "store_busy",
            "Connection profiles are being updated by another process.",
            3,
        ),
        ProfileMutationError::Lock(_)
        | ProfileMutationError::Persistence(_)
        | ProfileMutationError::Worker(_) => error(
            "persistence_failure",
            "Unable to save connection profiles.",
            5,
        ),
    }
}

fn read_password(
    password_env: Option<&str>,
    password_stdin: bool,
    url_contains_password: bool,
) -> Result<Option<SecretString>, ConnectionCliError> {
    if password_env.is_some() && password_stdin
        || url_contains_password && (password_env.is_some() || password_stdin)
    {
        return Err(error(
            "invalid_arguments",
            "Use only one password source: URL, --password-env, or --password-stdin.",
            2,
        ));
    }
    if let Some(name) = password_env {
        let value = std::env::var(name).map_err(|_| {
            error(
                "credential_input_missing",
                "The requested password environment variable is not set.",
                4,
            )
        })?;
        if value.len() > MAX_PASSWORD_BYTES {
            return Err(error(
                "credential_input_invalid",
                "Password input is too long.",
                4,
            ));
        }
        return Ok(Some(SecretString::from(value)));
    }
    if password_stdin {
        if io::stdin().is_terminal() {
            return Err(error(
                "credential_input_invalid",
                "--password-stdin requires a non-terminal input stream.",
                4,
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = io::stdin()
                .take((MAX_PASSWORD_BYTES + 3) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
        let mut bytes = receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                error(
                    "credential_input_invalid",
                    "Timed out waiting for password input.",
                    4,
                )
            })?
            .map_err(|_| {
                error(
                    "credential_input_invalid",
                    "Unable to read password input.",
                    4,
                )
            })?;
        if bytes.len() > MAX_PASSWORD_BYTES + 2 {
            return Err(error(
                "credential_input_invalid",
                "Password input is too long.",
                4,
            ));
        }
        if bytes.ends_with(b"\r\n") {
            bytes.truncate(bytes.len() - 2);
        } else if bytes.ends_with(b"\n") {
            bytes.pop();
        }
        let value = String::from_utf8(bytes).map_err(|_| {
            error(
                "credential_input_invalid",
                "Password input must be valid UTF-8.",
                4,
            )
        })?;
        if value.len() > MAX_PASSWORD_BYTES {
            return Err(error(
                "credential_input_invalid",
                "Password input is too long.",
                4,
            ));
        }
        return Ok(Some(SecretString::from(value)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::Cli;

    use super::read_password;

    #[test]
    fn connection_password_environment_value_is_never_returned_in_errors() {
        let error =
            read_password(Some("LAZYDB_TEST_MISSING_CONNECTION_SECRET"), false, false).unwrap_err();
        assert_eq!(error.code, "credential_input_missing");
        assert!(
            !error
                .message
                .contains("LAZYDB_TEST_MISSING_CONNECTION_SECRET")
        );
    }

    #[test]
    fn connection_add_requires_url_at_execution_boundary() {
        let cli = Cli::try_parse_from([
            "lazydb",
            "connections",
            "add",
            "--name",
            "demo",
            "--scope",
            "global",
        ])
        .unwrap();
        assert!(cli.url.is_none());
    }
}
