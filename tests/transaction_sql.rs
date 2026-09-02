use lazydb::model::transaction::{TransactionMode, TransactionState};
use lazydb::sql::{
    BeginRequest, SqlDialect, SqlRisk, TransactionControl, TransactionSqlClassification,
    TransactionSqlError, classify_transaction_batch, classify_transaction_sql,
    savepoint_requires_active_manual, validate_transaction_control,
};

fn control(sql: &str) -> TransactionControl {
    match classify_transaction_sql(sql, SqlDialect::Postgres) {
        TransactionSqlClassification::Control(control) => control,
        other => panic!("expected control, got {other:?}"),
    }
}

#[test]
fn accepts_only_bare_begin_forms_and_canonicalizes_them() {
    for sql in ["BEGIN", "BEGIN WORK", "START TRANSACTION"] {
        assert_eq!(
            classify_transaction_sql(sql, SqlDialect::Postgres),
            TransactionSqlClassification::Control(TransactionControl::Begin(
                BeginRequest::Canonical
            ))
        );
    }
    assert_eq!(BeginRequest::Canonical.canonical_sql(), "BEGIN");
    for sql in [
        "BEGIN ISOLATION LEVEL SERIALIZABLE",
        "BEGIN READ ONLY",
        "START TRANSACTION WITH CONSISTENT SNAPSHOT",
        "BEGIN AND CHAIN",
    ] {
        assert!(matches!(
            classify_transaction_sql(sql, SqlDialect::Postgres),
            TransactionSqlClassification::Unsupported(TransactionSqlError::UnsupportedOptions)
        ));
    }
}

#[test]
fn classifies_commit_end_rollback_and_savepoints() {
    assert_eq!(control("COMMIT"), TransactionControl::Commit);
    assert_eq!(control("END;"), TransactionControl::Commit);
    assert_eq!(control("ROLLBACK"), TransactionControl::Rollback);
    assert_eq!(
        control("ROLLBACK TO SAVEPOINT checkpoint"),
        TransactionControl::RollbackToSavepoint("checkpoint".into())
    );
    assert_eq!(
        control("SAVEPOINT checkpoint"),
        TransactionControl::Savepoint("checkpoint".into())
    );
    assert_eq!(
        control("RELEASE SAVEPOINT checkpoint"),
        TransactionControl::ReleaseSavepoint("checkpoint".into())
    );
}

#[test]
fn rejects_mixed_and_unsupported_controls() {
    assert!(matches!(
        classify_transaction_sql("BEGIN; SELECT 1", SqlDialect::Postgres),
        TransactionSqlClassification::Unsupported(TransactionSqlError::MixedControlAndData)
    ));
    for sql in [
        "COMMIT AND CHAIN",
        "ROLLBACK AND CHAIN",
        "SET TRANSACTION READ ONLY",
        "SET autocommit = 0",
    ] {
        assert!(matches!(
            classify_transaction_sql(sql, SqlDialect::Postgres),
            TransactionSqlClassification::Unsupported(
                TransactionSqlError::UnsupportedControl | TransactionSqlError::UnsupportedOptions
            )
        ));
    }
}

#[test]
fn savepoints_require_active_manual_transaction() {
    let savepoint = control("SAVEPOINT item");
    assert!(savepoint_requires_active_manual(
        &savepoint,
        TransactionMode::Manual,
        TransactionState::Idle
    ));
    assert!(!savepoint_requires_active_manual(
        &savepoint,
        TransactionMode::Manual,
        TransactionState::Active
    ));
    assert_eq!(
        validate_transaction_control(
            &savepoint,
            TransactionMode::Manual,
            TransactionState::Active,
            true
        ),
        Err(TransactionSqlError::ReadOnly)
    );
    let begin = control("BEGIN");
    assert_eq!(
        validate_transaction_control(
            &begin,
            TransactionMode::Manual,
            TransactionState::Active,
            false
        ),
        Err(TransactionSqlError::InvalidControl)
    );
}

#[test]
fn mysql_ddl_reports_implicit_commit() {
    assert_eq!(
        classify_transaction_sql("CREATE TABLE t (id INT)", SqlDialect::MySql),
        TransactionSqlClassification::Data {
            risk: SqlRisk::Ddl,
            mysql_implicit_commit: true,
        }
    );
    assert!(matches!(
        classify_transaction_sql("UPDATE t SET id = 1", SqlDialect::MySql),
        TransactionSqlClassification::Data {
            risk: SqlRisk::Dml,
            mysql_implicit_commit: false,
        }
    ));
}

#[test]
fn batch_controls_are_not_data() {
    assert_eq!(
        classify_transaction_batch("SAVEPOINT a; RELEASE SAVEPOINT a", SqlDialect::Postgres)
            .unwrap(),
        vec![
            TransactionControl::Savepoint("a".into()),
            TransactionControl::ReleaseSavepoint("a".into())
        ]
    );
}

#[test]
fn sql_server_go_batches_are_separate_and_ddl_is_transactional() {
    assert!(matches!(
        classify_transaction_sql(
            "SELECT 1\nGO\nUPDATE [dbo].[items] SET [id] = 2",
            SqlDialect::SqlServer
        ),
        TransactionSqlClassification::Unsupported(TransactionSqlError::MultipleStatements)
    ));
    assert_eq!(
        classify_transaction_sql("CREATE TABLE [dbo].[t] ([id] int)", SqlDialect::SqlServer),
        TransactionSqlClassification::Data {
            risk: SqlRisk::Ddl,
            mysql_implicit_commit: false,
        }
    );
}
