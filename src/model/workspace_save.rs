#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveStatus {
    Clean,
    Dirty,
    Saving,
    Failed,
    Closing,
}

impl SaveStatus {
    pub const fn is_dirty(self) -> bool {
        !matches!(self, Self::Clean)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::SaveStatus;

    #[test]
    fn only_clean_is_not_dirty() {
        assert!(!SaveStatus::Clean.is_dirty());
        assert!(SaveStatus::Dirty.is_dirty());
        assert!(SaveStatus::Saving.is_dirty());
        assert!(SaveStatus::Failed.is_dirty());
        assert!(SaveStatus::Closing.is_dirty());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveRevision {
    pub session: u64,
    pub revision: u64,
}

impl SaveRevision {
    pub const fn new(session: u64, revision: u64) -> Self {
        Self { session, revision }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveState {
    pub status: SaveStatus,
    pub current_revision: u64,
    pub acknowledged_revision: u64,
}

impl Default for SaveState {
    fn default() -> Self {
        Self {
            status: SaveStatus::Clean,
            current_revision: 0,
            acknowledged_revision: 0,
        }
    }
}

impl SaveState {
    pub fn offered(&mut self) -> Option<u64> {
        self.current_revision = self.current_revision.checked_add(1)?;
        self.status = SaveStatus::Saving;
        Some(self.current_revision)
    }

    pub fn succeeded(&mut self, revision: u64) {
        if revision < self.acknowledged_revision {
            return;
        }
        self.acknowledged_revision = revision;
        self.status = if revision == self.current_revision {
            SaveStatus::Clean
        } else {
            SaveStatus::Dirty
        };
    }

    pub fn failed(&mut self, revision: u64) {
        if revision >= self.current_revision && revision >= self.acknowledged_revision {
            self.status = SaveStatus::Failed;
        }
    }

    pub fn begin_closing(&mut self) {
        self.status = SaveStatus::Closing;
    }

    pub fn is_acknowledged(&self, revision: u64) -> bool {
        self.acknowledged_revision >= revision
    }
}

#[cfg(test)]
mod tests {
    use super::{SaveRevision, SaveState, SaveStatus};

    #[test]
    fn save_status_has_explicit_lifecycle_states() {
        assert_ne!(SaveStatus::Clean, SaveStatus::Failed);
        assert_eq!(
            SaveRevision::new(4, 9),
            SaveRevision {
                session: 4,
                revision: 9
            }
        );
    }

    #[test]
    fn stale_results_cannot_mark_a_newer_revision_clean() {
        let mut state = SaveState::default();
        assert_eq!(state.offered(), Some(1));
        assert_eq!(state.offered(), Some(2));
        state.succeeded(1);
        assert_eq!(state.status, SaveStatus::Dirty);
        state.succeeded(2);
        assert_eq!(state.status, SaveStatus::Clean);
    }

    #[test]
    fn closing_is_not_acknowledged_until_the_requested_revision_succeeds() {
        let mut state = SaveState::default();
        assert_eq!(state.offered(), Some(1));
        state.begin_closing();
        assert!(!state.is_acknowledged(1));
        state.succeeded(1);
        assert!(state.is_acknowledged(1));
    }

    #[test]
    fn an_old_failure_does_not_replace_newer_save_state() {
        let mut state = SaveState::default();
        assert_eq!(state.offered(), Some(1));
        assert_eq!(state.offered(), Some(2));
        state.failed(1);
        assert_eq!(state.status, SaveStatus::Saving);
        state.failed(2);
        assert_eq!(state.status, SaveStatus::Failed);
    }
}
