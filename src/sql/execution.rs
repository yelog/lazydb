#![allow(clippy::too_many_arguments)]

use uuid::Uuid;

use crate::{
    model::{
        execution_target::ExecutionTarget,
        transaction::{TransactionMode, TransactionState},
        workspace::ConnectionIdentity,
    },
    sql::{ScopeKind, ScopeSource, SqlDialect, SqlRisk, classify_sql},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionDraft {
    pub console_id: Uuid,
    pub query_generation: u64,
    pub connection: ConnectionIdentity,
    pub target: ExecutionTarget,
    pub transaction_generation: u64,
    pub document_revision: u64,
    pub scope: ScopeKind,
    pub source: ScopeSource,
    pub sql: String,
    pub dialect: SqlDialect,
    pub statement_count: usize,
    pub risks: Vec<SqlRisk>,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
}

impl ExecutionDraft {
    pub fn new(
        console_id: Uuid,
        query_generation: u64,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        transaction_generation: u64,
        document_revision: u64,
        scope: ScopeKind,
        source: ScopeSource,
        sql: String,
        dialect: SqlDialect,
        transaction_mode: TransactionMode,
        transaction_state: TransactionState,
    ) -> Self {
        let analysis = classify_sql(&sql, dialect);
        Self {
            console_id,
            query_generation,
            connection,
            target,
            transaction_generation,
            document_revision,
            scope,
            source,
            sql,
            dialect,
            statement_count: analysis.statement_count,
            risks: analysis.risks,
            transaction_mode,
            transaction_state,
        }
    }

    pub fn requires_confirmation(&self, always: bool) -> bool {
        always
            || self.scope == ScopeKind::FullBuffer
            || self.statement_count != 1
            || self.risks.iter().any(|risk| *risk != SqlRisk::ReadOnly)
    }

    pub fn has_mixed_transaction_control(&self) -> bool {
        self.risks.contains(&SqlRisk::TransactionControl)
            && self
                .risks
                .iter()
                .any(|risk| *risk != SqlRisk::TransactionControl)
    }

    pub fn has_transaction_control(&self) -> bool {
        self.risks.contains(&SqlRisk::TransactionControl)
    }
}
