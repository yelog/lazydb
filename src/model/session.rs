use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::{
    db::{ServerInfo, catalog_mutation::CatalogMutationCapabilities},
    identity::ConnectionIdentity,
    model::execution_target::ExecutionTarget,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Connecting,
    Connected,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    pub target: ExecutionTarget,
    pub identity: ConnectionIdentity,
    pub status: SessionStatus,
    pub server: Option<ServerInfo>,
    pub mutation_capabilities: CatalogMutationCapabilities,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<ExecutionTarget, SessionState>,
    attempts: HashMap<ExecutionTarget, SessionState>,
    retired: HashSet<ConnectionIdentity>,
    next_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionRequest {
    Existing(ConnectionIdentity),
    Started(ConnectionIdentity),
}

impl SessionRegistry {
    pub fn register_attempt(&mut self, target: ExecutionTarget, identity: ConnectionIdentity) {
        if self.retired.contains(&identity) {
            return;
        }
        self.advance_generation_to(identity.generation);
        if let Some(previous) = self.attempts.remove(&target) {
            self.retired.insert(previous.identity);
        }
        self.attempts.insert(
            target.clone(),
            SessionState {
                target,
                identity,
                status: SessionStatus::Connecting,
                server: None,
                mutation_capabilities: Default::default(),
                error: None,
            },
        );
    }

    pub fn advance_generation_to(&mut self, generation: u64) {
        self.next_generation = self.next_generation.max(generation);
    }

    pub fn start_attempt(&mut self, target: ExecutionTarget) -> Option<ConnectionIdentity> {
        self.next_generation = self.next_generation.checked_add(1)?;
        let identity = ConnectionIdentity {
            profile_id: target.profile_id,
            generation: self.next_generation,
        };
        if let Some(previous) = self.attempts.remove(&target) {
            self.retired.insert(previous.identity);
        }
        self.attempts.insert(
            target.clone(),
            SessionState {
                target,
                identity,
                status: SessionStatus::Connecting,
                server: None,
                mutation_capabilities: Default::default(),
                error: None,
            },
        );
        Some(identity)
    }

    pub fn request(&mut self, target: ExecutionTarget) -> Option<SessionRequest> {
        if let Some(session) = self.sessions.get(&target)
            && matches!(
                session.status,
                SessionStatus::Connected | SessionStatus::Connecting
            )
        {
            return Some(SessionRequest::Existing(session.identity));
        }

        if let Some(attempt) = self.attempts.get(&target)
            && attempt.status == SessionStatus::Connecting
        {
            return Some(SessionRequest::Existing(attempt.identity));
        }

        let identity = self.start_attempt(target)?;
        Some(SessionRequest::Started(identity))
    }

    pub fn force_reconnect(&mut self, target: ExecutionTarget) -> Option<ConnectionIdentity> {
        self.start_attempt(target)
    }

    pub fn get(&self, target: &ExecutionTarget) -> Option<&SessionState> {
        self.sessions
            .get(target)
            .or_else(|| self.attempts.get(target))
    }

    pub fn get_by_identity(&self, identity: ConnectionIdentity) -> Option<&SessionState> {
        self.sessions
            .values()
            .find(|session| session.identity == identity)
            .or_else(|| {
                self.attempts
                    .values()
                    .find(|session| session.identity == identity)
            })
    }

    pub fn is_retired(&self, identity: ConnectionIdentity) -> bool {
        self.retired.contains(&identity)
    }

    pub fn retire_identity(&mut self, identity: ConnectionIdentity) -> bool {
        self.retired.insert(identity);
        let target = self
            .sessions
            .iter()
            .find(|(_, session)| session.identity == identity)
            .map(|(target, _)| target.clone())
            .or_else(|| {
                self.attempts
                    .iter()
                    .find(|(_, session)| session.identity == identity)
                    .map(|(target, _)| target.clone())
            });
        target.is_some_and(|target| {
            self.sessions
                .get(&target)
                .is_some_and(|session| session.identity == identity)
                .then(|| self.sessions.remove(&target));
            if self
                .attempts
                .get(&target)
                .is_some_and(|session| session.identity == identity)
            {
                self.attempts.remove(&target);
            }
            true
        })
    }

    pub fn accept_success(
        &mut self,
        identity: ConnectionIdentity,
        server: ServerInfo,
        mutation_capabilities: CatalogMutationCapabilities,
    ) -> bool {
        let Some(target) = self
            .attempts
            .iter()
            .find(|(_, session)| session.identity == identity)
            .map(|(target, _)| target.clone())
        else {
            return false;
        };
        let Some(mut session) = self.attempts.remove(&target) else {
            return false;
        };
        session.status = SessionStatus::Connected;
        session.server = Some(server);
        session.mutation_capabilities = mutation_capabilities;
        session.error = None;
        self.sessions.insert(target, session);
        true
    }

    pub fn accept_failure(&mut self, identity: ConnectionIdentity, error: String) -> bool {
        let Some(target) = self
            .attempts
            .iter()
            .find(|(_, session)| session.identity == identity)
            .map(|(target, _)| target.clone())
        else {
            return false;
        };
        let Some(mut session) = self.attempts.remove(&target) else {
            return false;
        };
        session.status = SessionStatus::Failed;
        session.error = Some(error);
        self.sessions.entry(target).or_insert(session);
        true
    }

    pub fn remove_profile(&mut self, profile_id: Uuid) -> Vec<ConnectionIdentity> {
        let mut removed: Vec<ConnectionIdentity> = self
            .sessions
            .iter()
            .filter(|(target, _)| target.profile_id == profile_id)
            .map(|(_, session)| session.identity)
            .collect();
        removed.extend(
            self.attempts
                .iter()
                .filter(|(target, _)| target.profile_id == profile_id)
                .map(|(_, session)| session.identity),
        );
        self.retired.extend(removed.iter().copied());
        self.sessions
            .retain(|target, _| target.profile_id != profile_id);
        self.attempts
            .retain(|target, _| target.profile_id != profile_id);
        removed
    }

    pub fn identities_for_profile(&self, profile_id: Uuid) -> Vec<ConnectionIdentity> {
        self.sessions
            .values()
            .filter(|session| session.identity.profile_id == profile_id)
            .map(|session| session.identity)
            .chain(
                self.attempts
                    .values()
                    .filter(|session| session.identity.profile_id == profile_id)
                    .map(|session| session.identity),
            )
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SessionState> {
        self.sessions.values().chain(self.attempts.values())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{DatabaseKind, import_connection_url};

    fn target(name: &str) -> ExecutionTarget {
        let profile = import_connection_url(":memory:", Some(name))
            .unwrap()
            .profile;
        ExecutionTarget::from_profile(&profile)
    }

    fn server(target: &ExecutionTarget) -> ServerInfo {
        ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: target.database.clone(),
            current_user: None,
        }
    }

    #[test]
    fn requests_are_single_flight_per_target_but_independent_across_targets() {
        let first = target("first");
        let second = target("second");
        let mut registry = SessionRegistry::default();

        let SessionRequest::Started(first_identity) = registry.request(first.clone()).unwrap()
        else {
            panic!("first request should start");
        };
        assert!(matches!(
            registry.request(first.clone()),
            Some(SessionRequest::Existing(identity)) if identity == first_identity
        ));
        let SessionRequest::Started(second_identity) = registry.request(second.clone()).unwrap()
        else {
            panic!("second target should start independently");
        };
        assert_ne!(first_identity, second_identity);
    }

    #[test]
    fn late_result_for_one_target_does_not_replace_another_target() {
        let first = target("first");
        let second = target("second");
        let mut registry = SessionRegistry::default();
        let SessionRequest::Started(first_identity) = registry.request(first.clone()).unwrap()
        else {
            panic!()
        };
        let SessionRequest::Started(second_identity) = registry.request(second.clone()).unwrap()
        else {
            panic!()
        };

        assert!(registry.accept_success(second_identity, server(&second), Default::default()));
        assert!(registry.accept_success(first_identity, server(&first), Default::default()));
        assert_eq!(
            registry.get(&first).unwrap().status,
            SessionStatus::Connected
        );
        assert_eq!(
            registry.get(&second).unwrap().status,
            SessionStatus::Connected
        );
    }

    #[test]
    fn old_identity_is_not_accepted_after_failure() {
        let target = target("first");
        let mut registry = SessionRegistry::default();
        let SessionRequest::Started(identity) = registry.request(target.clone()).unwrap() else {
            panic!()
        };
        assert!(registry.accept_failure(identity, "offline".into()));
        assert!(!registry.accept_success(identity, server(&target), Default::default()));
    }
}
