//! Pure SQL text services.
//!
//! This module deliberately deals in UTF-8 byte offsets and does not depend on
//! the editor, terminal, or runtime layers.

mod analysis;
mod batch;
mod completion;
mod derived_result;
mod dialect;
mod execution;
mod format;
mod highlight;
mod identifier_match;
mod range;
mod relation_filter;
mod risk;
mod scope;
mod transaction;

pub use analysis::{AnalysisKey, LineIndex};
pub use batch::{SqlServerBatchError, split_sql_server_batches};
pub use completion::{
    CompletionCandidate, CompletionContext, CompletionDependencies, CompletionIndex,
    CompletionKind, CompletionScheduleKey, CompletionScore, complete, completion_dependencies,
    quote_identifier, relation_ids_for_completion, should_offer_completion,
};
pub use derived_result::{
    DerivedQueryError, PaginatedSql, bounded_query, build_derived_paginated_query,
    build_derived_query, build_paginated_query, derived_query_capable,
};
pub use dialect::SqlDialect;
pub use execution::ExecutionDraft;
pub use format::{FormatError, format_sql};
pub use highlight::{HighlightKind, HighlightSpan, highlight_sql, highlight_sql_ranges};
#[allow(unused_imports)]
pub(crate) use identifier_match::{identifier_match, identifier_match_positions};
pub use range::TextRange;
pub use relation_filter::{RelationFilterError, validate_relation_preview_options};
pub use risk::{SqlRisk, SqlRiskAggregate, SqlRiskAnalysis, classify_sql};
pub use scope::{
    ResolvedScope, ScopeKind, ScopeSelection, ScopeSource, resolve_scope, scan_statements,
};
pub use transaction::{
    BeginRequest, TransactionControl, TransactionSqlClassification, TransactionSqlError,
    classify_transaction_batch, classify_transaction_sql, savepoint_requires_active_manual,
    validate_transaction_control,
};
