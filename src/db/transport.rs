use std::{future::Future, net::SocketAddr, path::PathBuf};

use crate::{
    profile::ConnectionProfile,
    ssh::{SshError, TunnelLease},
};

use super::DatabaseError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportEndpoint {
    Direct,
    Tcp(SocketAddr),
    Unix(PathBuf),
    Socks5(SocketAddr),
}

#[derive(Clone, Debug)]
pub(crate) struct ConnectionRoute {
    pub(crate) endpoint: TransportEndpoint,
    pub(crate) lease: Option<TunnelLease>,
}

impl ConnectionRoute {
    pub(crate) fn direct() -> Self {
        Self {
            endpoint: TransportEndpoint::Direct,
            lease: None,
        }
    }

    pub(crate) fn require_direct(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        if profile.ssh.is_some() {
            return Err(DatabaseError::configuration(
                "SSH profiles require the managed database connector",
            ));
        }
        Ok(Self::direct())
    }

    pub(crate) fn ensure_alive(&self) -> Result<(), DatabaseError> {
        if let Some(lease) = &self.lease {
            lease
                .ensure_alive()
                .map_err(SshError::into_database_error)?;
        }
        Ok(())
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.lease.as_ref().is_none_or(TunnelLease::is_alive)
    }

    pub(crate) async fn guard<F, T>(&self, operation: F) -> Result<T, DatabaseError>
    where
        F: Future<Output = Result<T, DatabaseError>>,
    {
        let Some(lease) = &self.lease else {
            return operation.await;
        };
        self.ensure_alive()?;
        let failure = lease.failure();
        tokio::select! {
            biased;
            () = failure => Err(SshError::exited().into_database_error()),
            result = operation => {
                self.ensure_alive()?;
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;
    use crate::ssh::test_lease;

    fn routed(address: SocketAddr, lease: TunnelLease) -> ConnectionRoute {
        ConnectionRoute {
            endpoint: TransportEndpoint::Tcp(address),
            lease: Some(lease),
        }
    }

    #[tokio::test]
    async fn failed_route_rejects_an_operation_before_it_starts() {
        let (lease, failure) = test_lease();
        let route = routed("127.0.0.1:12345".parse().unwrap(), lease);
        failure();
        let started = Arc::new(AtomicBool::new(false));
        let operation_started = Arc::clone(&started);

        let error = route
            .guard(async move {
                operation_started.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await
            .expect_err("failed route ran an operation");

        assert!(!started.load(Ordering::SeqCst));
        assert_eq!(error.code.as_deref(), Some("ssh_tunnel_exited"));
    }

    #[tokio::test]
    async fn failure_drops_an_in_flight_operation() {
        let (lease, failure) = test_lease();
        let route = routed("127.0.0.1:12345".parse().unwrap(), lease);
        let dropped = Arc::new(AtomicBool::new(false));
        let operation_dropped = Arc::clone(&dropped);

        let task = tokio::spawn(async move {
            route
                .guard(async move {
                    let _drop_guard = DropFlag(operation_dropped);
                    std::future::pending::<Result<(), DatabaseError>>().await
                })
                .await
        });
        tokio::task::yield_now().await;
        failure();

        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("failed route did not cancel its in-flight operation")
            .expect("guard task panicked")
            .expect_err("failed route completed an in-flight operation");
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(error.code.as_deref(), Some("ssh_tunnel_exited"));
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}
