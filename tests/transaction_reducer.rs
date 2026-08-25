use lazydb::model::transaction::{
    TransactionEvent, TransactionMode, TransactionSnapshot, TransactionState, restore, transition,
};

#[test]
fn manual_mode_enters_lazily_and_starts_on_first_execution() {
    let snapshot = transition(
        TransactionSnapshot::default(),
        TransactionEvent::EnterManual,
    )
    .unwrap();
    assert_eq!(snapshot.mode, TransactionMode::Manual);
    assert_eq!(snapshot.state, TransactionState::Idle);
    assert_eq!(snapshot.generation, 0);

    let starting = transition(snapshot, TransactionEvent::Start).unwrap();
    assert_eq!(starting.state, TransactionState::Starting);
    assert_eq!(starting.generation, 1);
    assert_eq!(
        transition(starting, TransactionEvent::Started)
            .unwrap()
            .state,
        TransactionState::Active
    );
}

#[test]
fn commit_and_outer_rollback_return_to_idle_with_new_generation() {
    let active = TransactionSnapshot {
        mode: TransactionMode::Manual,
        state: TransactionState::Active,
        generation: 4,
    };
    let committing = transition(active, TransactionEvent::Commit).unwrap();
    assert_eq!(committing.state, TransactionState::Committing);
    let idle = transition(committing, TransactionEvent::Committed).unwrap();
    assert_eq!(idle.state, TransactionState::Idle);
    assert_eq!(idle.generation, 5);

    let rolling_back = transition(active, TransactionEvent::Rollback).unwrap();
    assert_eq!(rolling_back.state, TransactionState::RollingBack);
    assert_eq!(
        transition(rolling_back, TransactionEvent::RolledBack)
            .unwrap()
            .generation,
        5
    );
}

#[test]
fn aborted_allows_only_rollback_and_unknown_outcome_is_blocked() {
    let active = TransactionSnapshot {
        mode: TransactionMode::Manual,
        state: TransactionState::Active,
        generation: 1,
    };
    let aborted = transition(active, TransactionEvent::StatementFailed).unwrap();
    assert_eq!(aborted.state, TransactionState::Aborted);
    assert!(transition(aborted, TransactionEvent::Commit).is_err());
    assert_eq!(
        transition(aborted, TransactionEvent::Rollback)
            .unwrap()
            .state,
        TransactionState::RollingBack
    );

    let unknown = transition(
        transition(active, TransactionEvent::Commit).unwrap(),
        TransactionEvent::OutcomeUnknown,
    )
    .unwrap();
    assert_eq!(unknown.state, TransactionState::OutcomeUnknown);
    assert!(transition(unknown, TransactionEvent::Start).is_err());
    assert_eq!(
        restore(TransactionMode::Manual, 8).state,
        TransactionState::Idle
    );
    assert_eq!(restore(TransactionMode::Manual, 8).generation, 9);
}
