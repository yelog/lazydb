//! Pure SQL text services.
//!
//! This module deliberately deals in UTF-8 byte offsets and does not depend on
//! the editor, terminal, or runtime layers.

mod analysis;
mod batch;
mod builtins;
mod catalog_change;
mod completion;
mod derived_result;
mod diagnostics;
mod dialect;
pub mod embedded;
mod execution;
mod format;
mod highlight;
mod identifier_match;
pub mod mariadb;
pub mod oracle;
mod range;
mod relation_filter;
mod risk;
mod scope;
mod semantic;
mod statement_kind;
mod transaction;
mod type_name;

pub use analysis::{AnalysisKey, LineIndex};
pub use batch::{SqlServerBatchError, split_sql_server_batches};
pub use catalog_change::{
    CatalogChange, CatalogChangeImpact, CatalogChangeKind, CatalogChangeName,
    extract_catalog_change_impact,
};
pub use completion::{
    CompletionCandidate, CompletionContext, CompletionDependencies, CompletionIndex,
    CompletionInsertionMode, CompletionKind, CompletionScheduleKey, CompletionScore, complete,
    complete_with_mode, completion_dependencies, qualifier_segments_at, quote_identifier,
    relation_ids_for_completion, should_offer_completion, should_offer_completion_for_dialect,
};
pub use derived_result::{
    DerivedQueryError, PaginatedSql, bounded_query, build_derived_paginated_query,
    build_derived_query, build_paginated_query, derived_query_capable,
};
pub use diagnostics::{DiagnosticScheduleKey, SqlDiagnostic, diagnose_sql};
pub use dialect::SqlDialect;
pub use execution::ExecutionDraft;
pub use format::{FormatError, format_sql};
pub use highlight::{
    HighlightKind, HighlightSpan, SqlClauseKind, highlight_sql, highlight_sql_clause,
    highlight_sql_ranges,
};
#[allow(unused_imports)]
pub(crate) use identifier_match::{identifier_match, identifier_match_positions};
pub use range::TextRange;
pub use relation_filter::{
    RelationColumnSort, RelationFilterError, SortDirection, cycle_relation_column_sort,
    relation_column_sort_projection, validate_relation_preview_options,
};
pub use risk::{SqlRisk, SqlRiskAggregate, SqlRiskAnalysis, classify_sql};
pub(crate) use scope::scan_mysql_statements;
pub use scope::{
    ResolvedScope, ScopeKind, ScopeSelection, ScopeSource, resolve_scope, scan_statements,
};
pub use semantic::{
    CatalogCoverage, CatalogNamespace, CatalogSnapshot, RelationResolution, SemanticAnalysis,
    SemanticContext, analyze_semantics,
};
pub use statement_kind::{SqlStatementKind, classify_statement_kind};
pub use transaction::{
    BeginRequest, TransactionControl, TransactionSqlClassification, TransactionSqlError,
    classify_transaction_batch, classify_transaction_sql, savepoint_requires_active_manual,
    validate_transaction_control,
};
pub use type_name::short_type_name;
