use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    agent::{
        context::{AgentProfileScope, AgentProjectContext, VisibleAgentProfile},
        selection::{AgentError, SelectedAgentProfile, select_profile},
        types::{AgentConnection, AgentContext, AgentQueryResult, AgentTarget, QueryOutcomeJson},
    },
    db::DatabaseConnection,
    persistence::{
        credentials::CredentialResolver, local_credentials::LocalCredentialStore, paths::AppPaths,
        profiles::ProfileStore, secrets::NativeSecretStore,
    },
    profile::ConnectionProfile,
};

pub const DEFAULT_MAX_ROWS: usize = 500;
pub const DEFAULT_MAX_RESULT_BYTES: usize = 1024 * 1024;

pub struct AgentService {
    project: AgentProjectContext,
    profiles: Vec<ConnectionProfile>,
    credential_resolver: CredentialResolver,
    max_rows: usize,
    max_result_bytes: usize,
}

impl AgentService {
    pub fn load(project: Option<&Path>, config: Option<PathBuf>) -> Result<Self, AgentError> {
        let project = AgentProjectContext::resolve(project).map_err(|error| AgentError {
            code: super::selection::AgentErrorCode::NoVisibleConnections,
            message: error.to_string(),
        })?;
        let paths = AppPaths::discover().map_err(|error| AgentError {
            code: super::selection::AgentErrorCode::NoVisibleConnections,
            message: error.to_string(),
        })?;
        let profile_path = config.unwrap_or_else(|| paths.profiles_file());
        let profiles = ProfileStore::new(profile_path)
            .load()
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::NoVisibleConnections,
                message: error.to_string(),
            })?;
        let credentials = CredentialResolver::new(
            Arc::new(NativeSecretStore),
            LocalCredentialStore::from_paths(&paths, "lazydb"),
        );
        Ok(Self::new(project, profiles, credentials))
    }

    pub fn new(
        project: AgentProjectContext,
        profiles: Vec<ConnectionProfile>,
        credential_resolver: CredentialResolver,
    ) -> Self {
        Self {
            project,
            profiles,
            credential_resolver,
            max_rows: DEFAULT_MAX_ROWS,
            max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
        }
    }

    pub fn with_limits(mut self, max_rows: usize, max_result_bytes: usize) -> Self {
        self.max_rows = max_rows;
        self.max_result_bytes = max_result_bytes;
        self
    }

    pub fn visible_profiles(&self) -> Vec<VisibleAgentProfile<'_>> {
        self.project.visible_profiles(&self.profiles)
    }

    pub fn select(&self, selector: Option<&str>) -> Result<SelectedAgentProfile<'_>, AgentError> {
        let visible = self.visible_profiles();
        select_profile(&visible, selector)
    }

    pub fn connections(&self) -> Vec<AgentConnection> {
        self.visible_profiles()
            .into_iter()
            .map(|entry| project_connection(entry.profile, entry.scope))
            .collect()
    }

    pub fn context(&self, selector: Option<&str>) -> Result<AgentContext, AgentError> {
        let selected = self.select(selector)?;
        Ok(AgentContext {
            project_root: self.project.root().display().to_string(),
            project_name: self.project.project.display_name.clone(),
            connections: self.connections(),
            selected_connection: selected.profile.id.to_string(),
        })
    }

    pub async fn query(
        &self,
        selector: Option<&str>,
        sql: &str,
    ) -> Result<AgentQueryResult, AgentError> {
        let selected = self.select(selector)?;
        super::policy::authorize_query(selected.profile, sql).map_err(|error| AgentError {
            code: super::selection::AgentErrorCode::PolicyDenied,
            message: format!("query rejected: {error:?}"),
        })?;
        let password = self
            .credential_resolver
            .resolve_headless(selected.profile)
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::CredentialFailure,
                message: error.to_string(),
            })?;
        let connection = DatabaseConnection::connect(selected.profile, password.as_ref())
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::DatabaseFailure,
                message: error.to_string(),
            })?;
        let outcome = connection.execute(sql).await.map_err(|error| AgentError {
            code: super::selection::AgentErrorCode::DatabaseFailure,
            message: error.to_string(),
        });
        connection.close().await;
        let outcome = outcome?;
        let mut json = QueryOutcomeJson::from(outcome);
        bound_result(&mut json, self.max_rows, self.max_result_bytes)?;
        Ok(AgentQueryResult {
            target: AgentTarget {
                connection: project_connection(selected.profile, selected.scope),
                schema: selected.profile.default_schema.clone(),
            },
            outcome: json,
        })
    }

    pub async fn search_schema(
        &self,
        selector: Option<&str>,
        query: String,
        limit: usize,
    ) -> Result<super::catalog::SchemaSearchResult, AgentError> {
        let selected = self.select(selector)?;
        let password = self
            .credential_resolver
            .resolve_headless(selected.profile)
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::CredentialFailure,
                message: error.to_string(),
            })?;
        let connection = DatabaseConnection::connect(selected.profile, password.as_ref())
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::DatabaseFailure,
                message: error.to_string(),
            })?;
        let target = AgentTarget {
            connection: project_connection(selected.profile, selected.scope),
            schema: selected.profile.default_schema.clone(),
        };
        let result =
            super::catalog::search_schema(&connection, selected.profile, target, query, limit)
                .await;
        connection.close().await;
        result
    }

    pub async fn execute(
        &self,
        selector: Option<&str>,
        sql: &str,
        policy: super::policy::WritePolicy,
    ) -> Result<AgentQueryResult, AgentError> {
        let selected = self.select(selector)?;
        super::policy::authorize_write(selected.profile, policy, sql).map_err(|error| {
            AgentError {
                code: super::selection::AgentErrorCode::PolicyDenied,
                message: format!("write rejected: {error:?}"),
            }
        })?;
        let password = self
            .credential_resolver
            .resolve_headless(selected.profile)
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::CredentialFailure,
                message: error.to_string(),
            })?;
        let connection = DatabaseConnection::connect(selected.profile, password.as_ref())
            .await
            .map_err(|error| AgentError {
                code: super::selection::AgentErrorCode::DatabaseFailure,
                message: error.to_string(),
            })?;
        let result = connection.execute(sql).await.map_err(|error| AgentError {
            code: super::selection::AgentErrorCode::DatabaseFailure,
            message: error.to_string(),
        });
        connection.close().await;
        let outcome = result?;
        Ok(AgentQueryResult {
            target: AgentTarget {
                connection: project_connection(selected.profile, selected.scope),
                schema: selected.profile.default_schema.clone(),
            },
            outcome: QueryOutcomeJson::from(outcome),
        })
    }
}

fn project_connection(profile: &ConnectionProfile, scope: AgentProfileScope) -> AgentConnection {
    AgentConnection {
        id: profile.id.to_string(),
        name: profile.name.clone(),
        scope: match scope {
            AgentProfileScope::CurrentProject => "current_project",
            AgentProfileScope::Global => "global",
        }
        .into(),
        kind: profile.kind,
        environment: profile.environment,
        host: profile.host.clone(),
        port: profile.port,
        database: profile.database.clone(),
        default_schema: profile.default_schema.clone(),
        user: profile.user.clone(),
        read_only: profile.read_only,
    }
}

fn bound_result(
    outcome: &mut QueryOutcomeJson,
    max_rows: usize,
    max_bytes: usize,
) -> Result<(), AgentError> {
    let mut rows = 0;
    let mut bytes = 0;
    let mut truncated = false;
    for result in &mut outcome.result_sets {
        result.rows.retain(|row| {
            if rows >= max_rows {
                truncated = true;
                return false;
            }
            let cell_sizes = row
                .iter()
                .map(|cell| serde_json::to_vec(cell).map_or(0, |value| value.len()))
                .collect::<Vec<_>>();
            if cell_sizes.iter().any(|size| *size > max_bytes) {
                truncated = true;
                return false;
            }
            let row_bytes: usize = cell_sizes.into_iter().sum();
            if bytes + row_bytes > max_bytes {
                truncated = true;
                return false;
            }
            rows += 1;
            bytes += row_bytes;
            true
        });
    }
    outcome.row_count = rows;
    outcome.truncated = truncated;
    Ok(())
}
