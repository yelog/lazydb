use std::collections::{HashMap, HashSet};

use tokio_util::sync::CancellationToken;

use crate::model::{execution_target::ExecutionTarget, workspace::ConnectionIdentity};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConnectionKey {
    pub identity: ConnectionIdentity,
    pub target: ExecutionTarget,
}

impl ConnectionKey {
    pub(crate) fn new(identity: ConnectionIdentity, target: ExecutionTarget) -> Self {
        Self { identity, target }
    }
}

#[derive(Default)]
pub(crate) struct ConnectionAttempts {
    in_flight: HashSet<ConnectionKey>,
    cancelled: HashSet<ConnectionKey>,
    latest_by_target: HashMap<ExecutionTarget, ConnectionIdentity>,
    cancellations: HashMap<ConnectionKey, CancellationToken>,
}

impl ConnectionAttempts {
    pub(crate) fn start(&mut self, key: ConnectionKey) -> bool {
        if let Some(latest) = self.latest_by_target.get(&key.target).copied() {
            if latest.generation > key.identity.generation {
                return false;
            }
            if latest.generation < key.identity.generation {
                for previous in self
                    .in_flight
                    .iter()
                    .filter(|previous| previous.target == key.target)
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    self.cancel_key(&previous);
                }
            }
        }
        if self.in_flight.contains(&key) {
            return false;
        }
        self.latest_by_target
            .insert(key.target.clone(), key.identity);
        self.cancelled.remove(&key);
        self.in_flight.insert(key.clone());
        self.cancellations.insert(key, CancellationToken::new());
        true
    }

    pub(crate) fn cancel_matching(&mut self, identity: ConnectionIdentity) {
        let keys = self
            .in_flight
            .iter()
            .filter(|key| key.identity == identity)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.cancel_key(&key);
        }
    }

    pub(crate) fn cancel_key(&mut self, key: &ConnectionKey) {
        if self.in_flight.contains(key) {
            self.cancelled.insert(key.clone());
            if let Some(cancellation) = self.cancellations.get(key) {
                cancellation.cancel();
            }
        }
    }

    pub(crate) fn cancellation(&self, key: &ConnectionKey) -> Option<CancellationToken> {
        self.cancellations.get(key).cloned()
    }

    pub(crate) fn is_current(&self, key: &ConnectionKey) -> bool {
        self.in_flight.contains(key)
            && !self.cancelled.contains(key)
            && self.latest_by_target.get(&key.target) == Some(&key.identity)
    }

    pub(crate) fn finish(&mut self, key: &ConnectionKey) {
        self.in_flight.remove(key);
        self.cancelled.remove(key);
        self.cancellations.remove(key);
    }

    pub(crate) fn cancel_all(&mut self) {
        for cancellation in self.cancellations.values() {
            cancellation.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn key(profile_id: Uuid, generation: u64, database: &str) -> ConnectionKey {
        ConnectionKey::new(
            ConnectionIdentity {
                profile_id,
                generation,
            },
            ExecutionTarget {
                profile_id,
                database: database.to_owned(),
                schema: None,
            },
        )
    }

    #[test]
    fn single_flight_is_scoped_to_complete_target_key() {
        let profile_id = Uuid::new_v4();
        let first = key(profile_id, 1, "one");
        let second = key(profile_id, 1, "two");
        let mut attempts = ConnectionAttempts::default();

        assert!(attempts.start(first.clone()));
        assert!(!attempts.start(first.clone()));
        assert!(attempts.start(second.clone()));
        assert!(attempts.is_current(&first));
        assert!(attempts.is_current(&second));
    }

    #[test]
    fn cancellation_and_completion_do_not_cross_targets() {
        let profile_id = Uuid::new_v4();
        let first = key(profile_id, 1, "one");
        let second = key(profile_id, 1, "two");
        let mut attempts = ConnectionAttempts::default();

        assert!(attempts.start(first.clone()));
        assert!(attempts.start(second.clone()));
        attempts.cancel_key(&first);
        assert!(!attempts.is_current(&first));
        assert!(attempts.is_current(&second));
        attempts.finish(&first);
        assert!(!attempts.is_current(&first));
        assert!(attempts.is_current(&second));
    }

    #[test]
    fn newer_unrelated_attempt_does_not_reject_late_success_candidate() {
        let profile_id = Uuid::new_v4();
        let first = key(profile_id, 1, "one");
        let second = key(Uuid::new_v4(), 2, "two");
        let mut attempts = ConnectionAttempts::default();

        assert!(attempts.start(second));
        assert!(attempts.start(first.clone()));
        assert!(attempts.is_current(&first));
    }

    #[test]
    fn newer_same_target_attempt_supersedes_older_attempt() {
        let profile_id = Uuid::new_v4();
        let old = key(profile_id, 1, "same");
        let new = key(profile_id, 2, "same");
        let mut attempts = ConnectionAttempts::default();

        assert!(attempts.start(old.clone()));
        let old_cancellation = attempts.cancellation(&old).unwrap();
        assert!(attempts.start(new.clone()));
        assert!(old_cancellation.is_cancelled());
        assert!(!attempts.is_current(&old));
        assert!(attempts.is_current(&new));
        assert!(!attempts.start(old));
    }

    #[tokio::test]
    async fn explicit_cancellation_signals_in_flight_attempt_only() {
        let profile_id = Uuid::new_v4();
        let first = key(profile_id, 1, "one");
        let second = key(profile_id, 1, "two");
        let mut attempts = ConnectionAttempts::default();

        assert!(attempts.start(first.clone()));
        assert!(attempts.start(second.clone()));
        let first_cancellation = attempts.cancellation(&first).unwrap();
        let second_cancellation = attempts.cancellation(&second).unwrap();
        attempts.cancel_key(&first);

        assert!(first_cancellation.is_cancelled());
        assert!(!second_cancellation.is_cancelled());
        attempts.finish(&first);
        assert!(attempts.cancellation(&first).is_none());
    }
}
