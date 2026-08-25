use std::collections::VecDeque;

use uuid::Uuid;

use super::workspace::ConnectionIdentity;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionState {
    #[default]
    Idle,
    Starting,
    Active,
    Aborted,
    Committing,
    RollingBack,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionSnapshot {
    pub mode: TransactionMode,
    pub state: TransactionState,
    pub generation: u64,
}

impl Default for TransactionSnapshot {
    fn default() -> Self {
        Self {
            mode: TransactionMode::Auto,
            state: TransactionState::Idle,
            generation: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionExitChoice {
    Commit,
    Rollback,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeferredIntent {
    SetMode(TransactionMode),
    CloseConsole,
    SwitchConnection { profile_id: Uuid, generation: u64 },
    DeleteProfile { profile_id: Uuid, request_id: u64 },
    Disconnect { connection: ConnectionIdentity },
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredTransactionPrompt {
    pub console_id: Uuid,
    pub transaction_generation: u64,
    pub intent: DeferredIntent,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeferredIntentQueue {
    pub prompts: VecDeque<DeferredTransactionPrompt>,
}

impl DeferredIntentQueue {
    pub fn push(&mut self, prompt: DeferredTransactionPrompt) {
        self.prompts.push_back(prompt);
    }

    pub fn pop(&mut self) -> Option<DeferredTransactionPrompt> {
        self.prompts.pop_front()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationIntent {
    pub console_id: Uuid,
    pub query_generation: u64,
    pub transaction_generation: u64,
    pub connection: ConnectionIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionEvent {
    EnterManual,
    SetAuto,
    Start,
    Started,
    StartFailed,
    Commit,
    Committed,
    CommitFailed,
    Rollback,
    RolledBack,
    RollbackFailed,
    StatementFailed,
    ImplicitlyEnded,
    OutcomeUnknown,
    ClearOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionTransitionError {
    ActiveTransaction,
    InvalidState,
    AbortedTransaction,
    UnknownOutcome,
    TransactionRequired,
}

/// Applies one domain event without performing I/O or consulting application state.
pub fn transition(
    snapshot: TransactionSnapshot,
    event: TransactionEvent,
) -> Result<TransactionSnapshot, TransactionTransitionError> {
    use TransactionEvent::*;
    use TransactionMode::{Auto, Manual};
    use TransactionState::{
        Aborted, Active, Committing, Idle, OutcomeUnknown, RollingBack, Starting,
    };

    let mut next = snapshot;
    match event {
        EnterManual if snapshot.state == Idle => next.mode = Manual,
        SetAuto if snapshot.mode == Manual && snapshot.state == Idle => next.mode = Auto,
        SetAuto if snapshot.mode == Auto && snapshot.state == Idle => {}
        Start if snapshot.mode == Manual && snapshot.state == Idle => {
            next.state = Starting;
            next.generation = next.generation.saturating_add(1);
        }
        Started if snapshot.state == Starting => next.state = Active,
        StartFailed if snapshot.state == Starting => {
            next.state = Idle;
            next.generation = next.generation.saturating_add(1);
        }
        Commit if snapshot.mode == Manual && snapshot.state == Active => next.state = Committing,
        Committed if snapshot.state == Committing => {
            next.state = Idle;
            next.generation = next.generation.saturating_add(1);
        }
        CommitFailed if snapshot.state == Committing => next.state = Active,
        Rollback if snapshot.mode == Manual && matches!(snapshot.state, Active | Aborted) => {
            next.state = RollingBack
        }
        RolledBack if snapshot.state == RollingBack => {
            next.state = Idle;
            next.generation = next.generation.saturating_add(1);
        }
        RollbackFailed if snapshot.state == RollingBack => {
            next.state = OutcomeUnknown;
            next.generation = next.generation.saturating_add(1);
        }
        StatementFailed if snapshot.mode == Manual && snapshot.state == Active => {
            next.state = Aborted
        }
        ImplicitlyEnded if snapshot.mode == Manual && snapshot.state == Active => {
            next.state = Idle;
            next.generation = next.generation.saturating_add(1);
        }
        TransactionEvent::OutcomeUnknown if matches!(snapshot.state, Committing | RollingBack) => {
            next.state = OutcomeUnknown;
            next.generation = next.generation.saturating_add(1);
        }
        ClearOutcome if snapshot.state == OutcomeUnknown => {
            next.state = Idle;
            next.generation = next.generation.saturating_add(1);
        }
        _ if snapshot.state == OutcomeUnknown => {
            return Err(TransactionTransitionError::UnknownOutcome);
        }
        _ if snapshot.state == Aborted && matches!(event, Commit | Start) => {
            return Err(TransactionTransitionError::AbortedTransaction);
        }
        _ if matches!(event, SetAuto | EnterManual) => {
            return Err(TransactionTransitionError::ActiveTransaction);
        }
        _ if matches!(event, Start | Commit | Rollback) => {
            return Err(TransactionTransitionError::TransactionRequired);
        }
        _ => return Err(TransactionTransitionError::InvalidState),
    }
    Ok(next)
}

pub fn restore(mode: TransactionMode, persisted_generation: u64) -> TransactionSnapshot {
    TransactionSnapshot {
        mode,
        state: TransactionState::Idle,
        generation: persisted_generation.saturating_add(1),
    }
}
