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
fn cockroach_and_postgres_statement_errors_abort_manual_transactions() {
    for url in [
        "cockroachdb://root@localhost/defaultdb",
        "postgresql://postgres@localhost/defaultdb",
    ] {
        let profile = lazydb::profile::import_connection_url(url, Some("transaction-test"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = lazydb::app::App::new(Vec::new());
        app.profiles.push(profile);
        app.connection.profile_id = Some(profile_id);
        app.connection.generation = 1;
        app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
        let tab = app.active_console_mut();
        tab.transaction_mode = TransactionMode::Manual;
        tab.transaction_state = TransactionState::Active;
        tab.transaction_generation = 4;
        tab.generation = 2;
        let tab_id = tab.id;
        app.update(lazydb::action::Action::ManualQueryFailed {
            tab_id,
            query_generation: 2,
            transaction_generation: 4,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            message: "transaction statement failed (SQLSTATE 40001)".into(),
        });
        assert_eq!(
            app.active_console().transaction_state,
            TransactionState::Aborted,
            "{url}"
        );
    }
}

#[test]
fn cockroach_rejected_commit_requires_rollback_and_unknown_commit_is_blocked() {
    for (unknown, expected) in [
        (false, TransactionState::Aborted),
        (true, TransactionState::OutcomeUnknown),
    ] {
        let profile =
            lazydb::profile::import_connection_url("cockroachdb://root@localhost/defaultdb", None)
                .unwrap()
                .profile;
        let profile_id = profile.id;
        let mut app = lazydb::app::App::new(Vec::new());
        app.profiles.push(profile);
        app.connection.profile_id = Some(profile_id);
        app.connection.generation = 1;
        app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
        let tab = app.active_console_mut();
        tab.transaction_mode = TransactionMode::Manual;
        tab.transaction_state = TransactionState::Committing;
        tab.transaction_generation = 4;
        tab.generation = 2;
        let tab_id = tab.id;
        app.update(lazydb::action::Action::ManualCommitFailed {
            tab_id,
            query_generation: 2,
            transaction_generation: 4,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            message: "commit failed".into(),
            unknown,
        });
        assert_eq!(app.active_console().transaction_state, expected);
    }
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
