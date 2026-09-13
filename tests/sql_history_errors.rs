use lazydb::db::{DatabaseError, ErrorCategory, transaction::TransactionError};

#[test]
fn database_error_exposes_a_history_safe_structured_snapshot() {
    let error = DatabaseError {
        category: ErrorCategory::Constraint,
        code: Some("23505".into()),
        message: "duplicate key".into(),
        diagnostic: None,
    };

    let snapshot = error.history_snapshot();
    assert_eq!(snapshot.category, ErrorCategory::Constraint);
    assert_eq!(snapshot.code.as_deref(), Some("23505"));
    assert_eq!(snapshot.message, "duplicate key");
}

#[test]
fn transaction_error_snapshot_preserves_relation_diagnostic_without_detail() {
    let error = TransactionError::relation(lazydb::db::transaction::RelationMutationDiagnostic {
        category: lazydb::db::transaction::RelationMutationCategory::Constraint,
        sqlstate: Some("23505".into()),
        operation: "insert".into(),
        relation: "public.items".into(),
        context: Some("row mutation".into()),
        message: "duplicate key".into(),
    });

    let snapshot = error.history_snapshot();
    assert_eq!(snapshot.message, "duplicate key");
    assert_eq!(snapshot.code.as_deref(), Some("23505"));
    assert!(!snapshot.message.contains("DETAIL"));
}
