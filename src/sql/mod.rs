//! Pure SQL text services.
//!
//! This module deliberately deals in UTF-8 byte offsets and does not depend on
//! the editor, terminal, or runtime layers.

mod analysis;
mod completion;
mod dialect;
mod execution;
mod format;
mod highlight;
mod range;
mod relation_filter;
mod risk;
mod scope;
mod transaction;

pub use analysis::{AnalysisKey, LineIndex};
pub use completion::{
    CompletionCandidate, CompletionIndex, CompletionKind, CompletionScheduleKey, CompletionScore,
    complete, quote_identifier, should_offer_completion,
};
pub use dialect::SqlDialect;
pub use execution::ExecutionDraft;
pub use format::{FormatError, format_sql};
pub use highlight::{HighlightKind, HighlightSpan, highlight_sql};
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
