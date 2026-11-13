use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{OnceCell, Semaphore, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::{AppConfig, SshConfig},
    db::transport::TransportEndpoint,
    profile::{ConnectionProfile, SshOptions},
};

use super::{
    error::{SshError, SshErrorKind},
    openssh::{self, ForwardMode, LaunchOptions},
};

#[derive(Clone, Debug)]
pub(crate) enum TunnelStatus {
    Starting,
    Ready(TransportEndpoint),
    Failed(SshError),
    Stopped,
}

struct SharedTunnel {
    state: watch::Receiver<TunnelStatus>,
    stop: CancellationToken,
}

impl Drop for SharedTunnel {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[derive(Clone)]
pub struct TunnelLease {
    shared: Arc<SharedTunnel>,
}

impl std::fmt::Debug for TunnelLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelLease")
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

impl TunnelLease {
    pub fn is_alive(&self) -> bool {
        matches!(*self.shared.state.borrow(), TunnelStatus::Ready(_))
            && !self.shared.stop.is_cancelled()
    }

    pub fn ensure_alive(&self) -> Result<(), SshError> {
        match &*self.shared.state.borrow() {
            TunnelStatus::Ready(_) if !self.shared.stop.is_cancelled() => Ok(()),
            TunnelStatus::Failed(error) => Err(error.clone()),
            _ => Err(SshError::exited()),
        }
    }

    pub fn failure(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.shared.state.clone();
        let stop = self.shared.stop.clone();
        async move {
            loop {
                if matches!(
                    *state.borrow_and_update(),
                    TunnelStatus::Failed(_) | TunnelStatus::Stopped
                ) || stop.is_cancelled()
                {
                    return;
                }
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    changed = state.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn endpoint(&self) -> Result<TransportEndpoint, SshError> {
        self.ensure_alive()?;
        match &*self.shared.state.borrow() {
            TunnelStatus::Ready(endpoint) => Ok(endpoint.clone()),
            _ => Err(SshError::exited()),
        }
    }

    async fn ready(&self) -> Result<(), SshError> {
        let mut state = self.shared.state.clone();
        let stop = self.shared.stop.clone();
        loop {
            let status = state.borrow_and_update().clone();
            match status {
                TunnelStatus::Starting => {}
                TunnelStatus::Ready(_) => return self.ensure_alive(),
                TunnelStatus::Failed(error) => return Err(error),
                TunnelStatus::Stopped => return Err(SshError::cancelled()),
            }
            tokio::select! {
                biased;
                () = stop.cancelled() => return Err(SshError::cancelled()),
                changed = state.changed() => changed.map_err(|_| SshError::exited())?,
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TunnelKey {
    profile_id: Uuid,
    ssh: SshOptions,
    mode: ForwardMode,
    destination: Option<(String, u16)>,
}

#[derive(Default)]
struct Registry {
    tunnels: HashMap<TunnelKey, Weak<SharedTunnel>>,
    tasks: Vec<JoinHandle<()>>,
}

struct ManagerState {
    config: SshConfig,
    registry: Mutex<Registry>,
    program: Arc<OnceCell<PathBuf>>,
    starts: Arc<Semaphore>,
    closed: AtomicBool,
}

#[derive(Clone)]
pub struct TunnelManager {
    state: Arc<ManagerState>,
}

impl std::fmt::Debug for TunnelManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelManager")
            .field("active_tunnels", &self.active_count())
            .finish_non_exhaustive()
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new(AppConfig::default().connections.ssh)
    }
}

impl TunnelManager {
    pub fn new(config: SshConfig) -> Self {
        Self {
            state: Arc::new(ManagerState {
                config,
                registry: Mutex::new(Registry::default()),
                program: Arc::new(OnceCell::new()),
                starts: Arc::new(Semaphore::new(4)),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub fn active_count(&self) -> usize {
        self.state
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .tunnels
            .values()
            .filter_map(Weak::upgrade)
            .filter(|tunnel| {
                !tunnel.stop.is_cancelled()
                    && matches!(
                        *tunnel.state.borrow(),
                        TunnelStatus::Starting | TunnelStatus::Ready(_)
                    )
            })
            .count()
    }

    pub async fn acquire(&self, profile: &ConnectionProfile) -> Result<TunnelLease, SshError> {
        let options = LaunchOptions::from_profile(profile, self.state.config)?;
        if options.config.connect_timeout_seconds == 0
            || options.config.connect_timeout_seconds > 300
            || options.config.server_alive_interval_seconds == 0
            || options.config.server_alive_interval_seconds > 3600
            || options.config.server_alive_count_max == 0
            || options.config.server_alive_count_max > 10
        {
            return Err(SshError::new(
                SshErrorKind::Configuration,
                "SSH timeout or keepalive configuration is outside the supported range",
            ));
        }
        let key = TunnelKey {
            profile_id: profile.id,
            ssh: options.ssh.clone(),
            mode: options.mode,
            destination: options.destination.clone(),
        };
        let shared = {
            let mut registry = self
                .state
                .registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.state.closed.load(Ordering::Acquire) {
                return Err(SshError::cancelled());
            }
            let existing = registry
                .tunnels
                .get(&key)
                .and_then(Weak::upgrade)
                .filter(|existing| {
                    !existing.stop.is_cancelled()
                        && matches!(
                            *existing.state.borrow(),
                            TunnelStatus::Starting | TunnelStatus::Ready(_)
                        )
                });
            if let Some(existing) = existing {
                existing
            } else {
                registry.tasks.retain(|task| !task.is_finished());
                registry.tunnels.retain(|_, weak| weak.strong_count() > 0);
                let (sender, receiver) = watch::channel(TunnelStatus::Starting);
                let stop = CancellationToken::new();
                let shared = Arc::new(SharedTunnel {
                    state: receiver,
                    stop: stop.clone(),
                });
                let program = self.state.program.clone();
                let starts = self.state.starts.clone();
                registry.tasks.push(tokio::spawn(async move {
                    let result =
                        start_tunnel(program, starts, options, sender.clone(), stop.clone()).await;
                    sender.send_replace(if stop.is_cancelled() {
                        TunnelStatus::Stopped
                    } else {
                        TunnelStatus::Failed(result.err().unwrap_or_else(SshError::exited))
                    });
                }));
                registry.tunnels.insert(key, Arc::downgrade(&shared));
                shared
            }
        };
        let lease = TunnelLease { shared };
        lease.ready().await?;
        Ok(lease)
    }

    pub async fn shutdown(&self) {
        self.state.closed.store(true, Ordering::Release);
        let mut tasks = {
            let mut registry = self
                .state
                .registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for tunnel in registry.tunnels.values().filter_map(Weak::upgrade) {
                tunnel.stop.cancel();
            }
            registry.tunnels.clear();
            std::mem::take(&mut registry.tasks)
        };
        if tokio::time::timeout(
            Duration::from_secs(3),
            futures_util::future::join_all(tasks.iter_mut()),
        )
        .await
        .is_err()
        {
            for task in &tasks {
                task.abort();
            }
            let _ = futures_util::future::join_all(tasks).await;
        }
    }
}

#[cfg(test)]
pub(crate) fn test_lease() -> (TunnelLease, impl FnOnce()) {
    let (sender, receiver) = watch::channel(TunnelStatus::Ready(TransportEndpoint::Tcp(
        "127.0.0.1:12345".parse().expect("test tunnel address"),
    )));
    let lease = TunnelLease {
        shared: Arc::new(SharedTunnel {
            state: receiver,
            stop: CancellationToken::new(),
        }),
    };
    (lease, move || {
        sender.send_replace(TunnelStatus::Failed(SshError::exited()));
    })
}

async fn start_tunnel(
    program: Arc<OnceCell<PathBuf>>,
    starts: Arc<Semaphore>,
    options: LaunchOptions,
    sender: watch::Sender<TunnelStatus>,
    stop: CancellationToken,
) -> Result<(), SshError> {
    let program = tokio::select! {
        biased;
        _ = stop.cancelled() => return Err(SshError::cancelled()),
        program = program.get_or_try_init(|| async {
            tokio::task::spawn_blocking(openssh::find_program).await.map_err(|_| {
                SshError::new(SshErrorKind::Unavailable, "OpenSSH client discovery failed")
            })?
        }) => program?.clone(),
    };
    let permit = tokio::select! {
        biased;
        _ = stop.cancelled() => return Err(SshError::cancelled()),
        permit = starts.acquire_owned() => permit.map_err(|_| SshError::cancelled())?,
    };
    let mut state = sender.subscribe();
    let running = openssh::supervise(program, options, sender, stop);
    tokio::pin!(running);
    tokio::select! {
        result = &mut running => result,
        _ = async {
            while matches!(*state.borrow_and_update(), TunnelStatus::Starting) {
                if state.changed().await.is_err() {
                    break;
                }
            }
        } => {
            drop(permit);
            running.await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tunnel() -> (TunnelLease, watch::Sender<TunnelStatus>) {
        let (sender, receiver) = watch::channel(TunnelStatus::Ready(TransportEndpoint::Tcp(
            "127.0.0.1:12345".parse().unwrap(),
        )));
        let lease = TunnelLease {
            shared: Arc::new(SharedTunnel {
                state: receiver,
                stop: CancellationToken::new(),
            }),
        };
        (lease, sender)
    }

    #[tokio::test]
    async fn failure_notification_does_not_keep_a_tunnel_lease_alive() {
        let (sender, receiver) = watch::channel(TunnelStatus::Ready(TransportEndpoint::Tcp(
            "127.0.0.1:12345".parse().unwrap(),
        )));
        let stop = CancellationToken::new();
        let lease = TunnelLease {
            shared: Arc::new(SharedTunnel {
                state: receiver,
                stop: stop.clone(),
            }),
        };
        let failure = lease.failure();
        let clone = lease.clone();
        drop(lease);
        assert!(!stop.is_cancelled());
        drop(clone);
        assert!(stop.is_cancelled());
        sender.send_replace(TunnelStatus::Stopped);
        tokio::time::timeout(Duration::from_secs(1), failure)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn failed_lease_cannot_supply_its_old_endpoint() {
        let (lease, sender) = test_tunnel();
        assert!(lease.is_alive());
        sender.send_replace(TunnelStatus::Failed(SshError::exited()));
        assert!(!lease.is_alive());
        assert!(lease.endpoint().is_err());
    }

    #[tokio::test]
    async fn cancellation_wakes_failure_waiters_without_a_state_update() {
        let (lease, _sender) = test_tunnel();
        let failure = lease.failure();
        let stop = lease.shared.stop.clone();
        let waiter = tokio::spawn(failure);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("cancelled tunnel left a failure waiter pending")
            .expect("failure waiter task panicked");
    }

    #[tokio::test]
    async fn cancelled_lease_does_not_wait_for_a_state_update() {
        let (sender, receiver) = watch::channel(TunnelStatus::Starting);
        let stop = CancellationToken::new();
        let lease = TunnelLease {
            shared: Arc::new(SharedTunnel {
                state: receiver,
                stop: stop.clone(),
            }),
        };
        stop.cancel();
        let error = tokio::time::timeout(Duration::from_secs(1), lease.ready())
            .await
            .expect("cancelled lease waited for a tunnel state update")
            .expect_err("cancelled lease became ready");
        assert_eq!(error.kind, SshErrorKind::Cancelled);
        drop(sender);
    }
}
