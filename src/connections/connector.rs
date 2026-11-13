use std::time::Duration;

use secrecy::SecretString;

use crate::{
    config::{AppConfig, SshConfig},
    db::{DatabaseConnection, DatabaseError, ErrorCategory, transport::ConnectionRoute},
    model::execution_target::ExecutionTarget,
    profile::ConnectionProfile,
    ssh::{SshError, TunnelManager},
};

#[derive(Clone, Debug)]
pub struct DatabaseConnector {
    tunnels: TunnelManager,
    config: SshConfig,
}

impl Default for DatabaseConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl DatabaseConnector {
    pub fn new() -> Self {
        Self::with_ssh_config(AppConfig::default().connections.ssh)
    }

    pub fn with_ssh_config(config: SshConfig) -> Self {
        Self {
            tunnels: TunnelManager::new(config),
            config,
        }
    }

    pub fn connect_timeout(&self, profile: &ConnectionProfile) -> Duration {
        Duration::from_secs(if profile.ssh.is_some() {
            self.config.connect_timeout_seconds
        } else {
            10
        })
    }

    pub fn active_tunnel_count(&self) -> usize {
        self.tunnels.active_count()
    }

    pub async fn connect(
        &self,
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<DatabaseConnection, DatabaseError> {
        if profile.ssh.is_none() {
            return DatabaseConnection::connect(profile, password).await;
        }
        profile
            .validate_ssh()
            .map_err(DatabaseError::configuration)?;
        let connect = async {
            let lease = self
                .tunnels
                .acquire(profile)
                .await
                .map_err(SshError::into_database_error)?;
            let route = ConnectionRoute {
                endpoint: lease.endpoint().map_err(SshError::into_database_error)?,
                lease: Some(lease),
            };
            DatabaseConnection::connect_routed(profile, password, route).await
        };
        tokio::time::timeout(self.connect_timeout(profile), connect)
            .await
            .map_err(|_| DatabaseError {
                category: ErrorCategory::Network,
                code: Some("connection_timeout".to_owned()),
                message: "SSH and database connection exceeded the connection timeout".to_owned(),
                diagnostic: None,
            })?
    }

    pub async fn connect_target(
        &self,
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
        target: &ExecutionTarget,
    ) -> Result<DatabaseConnection, DatabaseError> {
        let configured = target.apply_to_profile(profile).ok_or_else(|| {
            DatabaseError::configuration("execution target is invalid for this profile")
        })?;
        self.connect(&configured, password).await
    }

    pub async fn shutdown(&self) {
        self.tunnels.shutdown().await;
    }
}
