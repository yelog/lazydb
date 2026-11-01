use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

use crate::db::catalog::{CatalogEntry, CatalogId, CatalogKind, CatalogMetadata};
use crate::profile::CatalogScope;

use super::builtins::{Builtin, default_value_builtins, expression_builtins};
use super::identifier_match::{
    IdentifierMatch, compact_identifier, fold_identifier, identifier_match,
};
use super::scope::scan_statements;
use super::{SqlDialect, TextRange};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionScheduleKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub cursor: usize,
    pub connection: Option<crate::model::workspace::ConnectionIdentity>,
    pub target: Option<crate::model::execution_target::ExecutionTarget>,
    pub catalog_generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompletionContext<'a> {
    pub database: Option<&'a str>,
    pub schema: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CompletionInsertionMode {
    /// Prepend missing database/schema components based on the connection context.
    #[default]
    Contextual,
    /// Only insert the object name, never rewriting the surrounding path.
    CurrentSegment,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CompletionKind {
    Keyword,
    BuiltinExpression,
    DataType,
    Database,
    Schema,
    Table,
    View,
    Column,
    Index,
    Constraint,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Type,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CompletionScore {
    pub context: u8,
    pub name_match: u8,
    pub schema: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub replace: TextRange,
    pub score: CompletionScore,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionDependencies {
    pub relation_children: Vec<CatalogId>,
}

#[derive(Clone, Debug, Default)]
pub struct CompletionIndex {
    by_name: BTreeMap<String, Vec<usize>>,
    by_compact_name: BTreeMap<String, Vec<usize>>,
    children: HashMap<CatalogId, Vec<usize>>,
    entries: Vec<CatalogEntry>,
}

impl CompletionIndex {
    pub fn new(entries: &[CatalogEntry]) -> Self {
        let mut index = Self::default();
        index.replace(entries);
        index
    }

    pub fn replace(&mut self, entries: &[CatalogEntry]) {
        self.entries = accepted_entries(entries, None);
        self.rebuild();
    }

    pub fn append(&mut self, entries: &[CatalogEntry]) {
        self.entries.extend(accepted_entries(entries, None));
        self.deduplicate();
        self.rebuild();
    }

    pub fn replace_scoped(&mut self, entries: &[CatalogEntry], scope: &CatalogScope) {
        self.entries = accepted_entries(entries, Some(scope));
        self.rebuild();
    }

    pub fn append_scoped(&mut self, entries: &[CatalogEntry], scope: &CatalogScope) {
        self.entries.retain(|entry| entry_in_scope(entry, scope));
        self.entries.extend(accepted_entries(entries, Some(scope)));
        self.deduplicate();
        self.rebuild();
    }

    pub fn remove_ids(&mut self, ids: &HashSet<CatalogId>) {
        self.entries.retain(|entry| !ids.contains(&entry.id));
        self.rebuild();
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn relation_columns(&self, relation: &CatalogId) -> impl Iterator<Item = &CatalogEntry> {
        self.children
            .get(relation)
            .into_iter()
            .flatten()
            .filter_map(|position| self.entries.get(*position))
            .filter(|entry| entry.kind == CatalogKind::Column)
    }

    fn rebuild(&mut self) {
        self.by_name.clear();
        self.by_compact_name.clear();
        self.children.clear();
        for (position, entry) in self.entries.iter().enumerate() {
            self.by_name
                .entry(fold_identifier(&entry.qualified_name.object))
                .or_default()
                .push(position);
            self.by_compact_name
                .entry(compact_identifier(&entry.qualified_name.object))
                .or_default()
                .push(position);
            if let Some(parent) = &entry.parent_id {
                self.children
                    .entry(parent.clone())
                    .or_default()
                    .push(position);
            }
        }
    }

    fn deduplicate(&mut self) {
        let mut seen = std::collections::HashSet::with_capacity(self.entries.len());
        self.entries.retain(|entry| seen.insert(entry.id.clone()));
    }
}

fn accepted_entries(entries: &[CatalogEntry], scope: Option<&CatalogScope>) -> Vec<CatalogEntry> {
    let mut seen = std::collections::HashSet::with_capacity(entries.len());
    entries
        .iter()
        .filter(|entry| completion_kind(entry.kind).is_some())
        .filter(|entry| scope.is_none_or(|scope| entry_in_scope(entry, scope)))
        .filter(|entry| seen.insert(entry.id.clone()))
        .cloned()
        .collect()
}

fn entry_in_scope(entry: &CatalogEntry, scope: &CatalogScope) -> bool {
    match (
        entry.qualified_name.database.as_deref(),
        entry.qualified_name.schema.as_deref(),
    ) {
        (Some(database), Some(schema)) => scope.allows_schema(database, schema),
        (Some(database), None) => scope.allows_database(database),
        (None, _) => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Context {
    Statement,
    Insert,
    UpdateSet,
    Delete(DeleteContext),
    Relation,
    AssignmentTarget,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
    Ddl(DdlContext),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteContext {
    From,
    AfterTarget,
    Alias,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DdlContext {
    CreateObjectKind,
    AlterObjectKind,
    DropObjectKind,
    ExistingObject(DdlObjectTarget),
    CreateIndexTarget,
    ColumnType,
    ColumnConstraint,
    AlterColumnDefinition,
    AlterColumnType(bool),
    TableConstraint,
    AlterTableAction,
    ExistingColumn,
    ExistingConstraint,
    ExistingIndex,
    ReferenceRelation,
    ReferenceColumn,
    DefaultValue,
    NewColumnName,
    RenameColumnTo,
    NewObjectName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DdlObjectTarget {
    Database,
    Schema,
    Table,
    View,
    Index,
    Trigger,
    Sequence,
    Type,
    Function,
    Procedure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionContext {
    Projection,
    Predicate,
    Grouping,
    Ordering,
    Returning,
    AssignmentValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OrderingStage {
    Expression,
    Direction,
    AfterDirection,
    NullPlacement,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CompletionTokenKind {
    Word(String),
    Literal,
    Operator,
    Equals,
    Star,
    Dot,
    Comma,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionToken {
    kind: CompletionTokenKind,
    start: usize,
    end: usize,
    depth: usize,
    scope_start: Option<usize>,
    quoted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RelationBinding {
    name: Vec<String>,
    alias: Option<String>,
    depth: usize,
    scope_start: Option<usize>,
}

pub fn complete(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Vec<CompletionCandidate> {
    complete_with_mode(
        text,
        cursor,
        dialect,
        index,
        completion_context,
        CompletionInsertionMode::Contextual,
    )
}

pub fn complete_with_mode(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
    insertion_mode: CompletionInsertionMode,
) -> Vec<CompletionCandidate> {
    let cursor = cursor.min(text.len());
    let (identifier_replace, prefix, qualifiers, quoted_segment) =
        identifier_at(text, cursor, dialect);
    let replace = if insertion_mode == CompletionInsertionMode::Contextual {
        TextRange::new(identifier_replace.start, cursor)
    } else {
        identifier_replace
    };
    let (statement, statement_cursor) = current_statement(text, replace.start, dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    let context = context_at(
        &tokens,
        statement_cursor,
        active_scopes.last().copied().flatten(),
        dialect,
        &prefix,
    );
    let ordering_stage =
        matches!(context, Context::Expression(ExpressionContext::Ordering)).then(|| {
            ordering_stage(
                &tokens,
                replace.start,
                active_scopes.last().copied().flatten(),
            )
        });
    let projection_complete = context == Context::Expression(ExpressionContext::Projection)
        && projection_is_complete(
            &tokens,
            statement_cursor,
            active_scopes.last().copied().flatten(),
        );
    let bindings = visible_relation_bindings(&tokens, &active_scopes);
    let visible_relations = bindings
        .iter()
        .flat_map(|binding| relation_ids(index, binding, completion_context))
        .collect::<HashSet<_>>();
    let mut candidates = Vec::new();
    let folded_prefix = fold_identifier(&prefix);
    let child_parent = ddl_child_parent(
        &tokens,
        statement_cursor,
        context,
        index,
        completion_context,
    );
    let candidate_indexes = if context == Context::AssignmentTarget {
        let targets = assignment_target_ids(
            &tokens,
            statement_cursor,
            dialect,
            index,
            completion_context,
        );
        let qualified = (!qualifiers.is_empty()).then(|| {
            qualified_candidate_indices(
                index,
                &qualifiers,
                &folded_prefix,
                &bindings,
                completion_context,
                dialect,
            )
        });
        if dialect == SqlDialect::Postgres && qualified.is_some() {
            Vec::new()
        } else {
            relation_child_candidate_indices(index, &targets)
                .into_iter()
                .filter(|position| {
                    index.entries[*position].kind == CatalogKind::Column
                        && qualified
                            .as_ref()
                            .is_none_or(|qualified| qualified.contains(position))
                })
                .collect()
        }
    } else if let Some((parent, child_kind)) = child_parent {
        index
            .children
            .get(&parent)
            .into_iter()
            .flatten()
            .copied()
            .filter(|position| completion_kind(index.entries[*position].kind) == Some(child_kind))
            .collect()
    } else if matches!(context, Context::Ddl(DdlContext::ExistingObject(_))) {
        ddl_candidate_indices(index, &qualifiers, &folded_prefix)
    } else {
        qualified_candidate_indices(
            index,
            &qualifiers,
            &folded_prefix,
            &bindings,
            completion_context,
            dialect,
        )
    };
    for node_index in candidate_indexes {
        let entry = &index.entries[node_index];
        let Some(kind) = completion_kind(entry.kind) else {
            continue;
        };
        if !catalog_kind_allowed(context, kind)
            || matches!(
                ordering_stage,
                Some(
                    OrderingStage::Direction
                        | OrderingStage::AfterDirection
                        | OrderingStage::NullPlacement
                        | OrderingStage::Complete,
                )
            )
        {
            continue;
        }
        if kind == CompletionKind::Column
            && context != Context::AssignmentTarget
            && qualifiers.is_empty()
            && !bindings.is_empty()
            && !entry
                .parent_id
                .as_ref()
                .is_some_and(|parent| visible_relations.contains(parent))
        {
            continue;
        }
        if dialect == SqlDialect::Sqlite
            && matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        {
            continue;
        }
        if matches!(dialect, SqlDialect::MySql | SqlDialect::Sqlite)
            && kind == CompletionKind::Schema
            && entry.kind == CatalogKind::Schema
            && entry.id.native_path.len() == 2
            && entry
                .id
                .native_path
                .first()
                .zip(entry.id.native_path.get(1))
                .is_some_and(|(database, schema)| database.eq_ignore_ascii_case(schema))
        {
            continue;
        }
        let name = &entry.qualified_name.object;
        let Some(name_match) = identifier_match(name, &prefix) else {
            continue;
        };
        let context_score = match (context, kind) {
            (Context::Relation, CompletionKind::Table | CompletionKind::View)
            | (Context::Qualifier, CompletionKind::Column)
            | (Context::AssignmentTarget, CompletionKind::Column)
            | (Context::Expression(_), CompletionKind::Column)
            | (Context::Routine, CompletionKind::Function | CompletionKind::Procedure) => 3,
            (_, CompletionKind::Keyword) => 1,
            _ => 2,
        };
        let schema_score = u8::from(completion_context.schema.is_some_and(|schema| {
            entry
                .id
                .native_path
                .iter()
                .any(|part| part.eq_ignore_ascii_case(schema))
        }));
        candidates.push(CompletionCandidate {
            label: display_text(name),
            insert_text: if matches!(kind, CompletionKind::Table | CompletionKind::View) {
                match insertion_mode {
                    CompletionInsertionMode::Contextual => {
                        relation_insert_text(entry, completion_context, dialect, &qualifiers)
                    }
                    CompletionInsertionMode::CurrentSegment => {
                        if quoted_segment {
                            quote_identifier(name, dialect)
                        } else {
                            quote_relation_component(name, dialect)
                        }
                    }
                }
            } else {
                column_insert_text(name, dialect, quoted_segment)
            },
            kind,
            detail: if matches!(kind, CompletionKind::Table | CompletionKind::View) {
                relation_detail(entry)
            } else if kind == CompletionKind::Schema {
                schema_detail(entry)
            } else {
                completion_detail(entry).map(|detail| display_text(&detail))
            },
            replace,
            score: CompletionScore {
                context: context_score,
                name_match: match name_match {
                    IdentifierMatch::CompactPrefix => 1,
                    IdentifierMatch::Prefix => 2,
                    IdentifierMatch::Exact => 3,
                },
                schema: schema_score,
            },
        });
    }
    if qualifiers.is_empty() {
        // A complete UPDATE prefix can end its SET clause; incomplete expressions cannot.
        if context == Context::Expression(ExpressionContext::AssignmentValue)
            && !quoted_segment
            && "where".starts_with(&folded_prefix)
            && sqlparser::parser::Parser::parse_sql(
                super::dialect::parser_dialect(dialect),
                &statement[..statement_cursor],
            )
            .is_ok_and(|statements| match statements.as_slice() {
                [sqlparser::ast::Statement::Update(_)] => true,
                [sqlparser::ast::Statement::Query(query)] => {
                    matches!(query.body.as_ref(), sqlparser::ast::SetExpr::Update(_))
                }
                _ => false,
            })
        {
            candidates.push(CompletionCandidate {
                label: "WHERE".to_owned(),
                insert_text: "WHERE".to_owned(),
                kind: CompletionKind::Keyword,
                detail: None,
                replace,
                score: CompletionScore {
                    context: 4,
                    name_match: 2,
                    schema: 0,
                },
            });
        }
        for keyword in
            keywords_for_completion(context, dialect, projection_complete, ordering_stage)
        {
            if keyword.to_lowercase().starts_with(&folded_prefix) {
                candidates.push(CompletionCandidate {
                    label: (*keyword).to_owned(),
                    insert_text: (*keyword).to_owned(),
                    kind: CompletionKind::Keyword,
                    detail: None,
                    replace,
                    score: CompletionScore {
                        context: match (context, projection_complete, *keyword) {
                            (Context::Expression(ExpressionContext::Projection), true, "FROM") => 4,
                            (Context::Statement | Context::Insert | Context::UpdateSet, _, _) => 4,
                            (Context::Delete(_), _, _) => 4,
                            (Context::Expression(_), _, _) => 2,
                            (Context::Relation | Context::Routine, _, _) => 1,
                            (Context::Qualifier | Context::AssignmentTarget, _, _) => 0,
                            (Context::Ddl(_), _, _) => 4,
                        },
                        name_match: 2,
                        schema: 0,
                    },
                });
            }
        }
        if context == Context::Ddl(DdlContext::AlterTableAction)
            && let Some(keyword) =
                alter_table_action_continuation(&tokens, statement_cursor, &prefix)
            && !candidates
                .iter()
                .any(|candidate| candidate.label == keyword)
        {
            candidates.push(CompletionCandidate {
                label: keyword.to_owned(),
                insert_text: keyword.to_owned(),
                kind: CompletionKind::Keyword,
                detail: None,
                replace,
                score: CompletionScore {
                    context: 4,
                    name_match: 2,
                    schema: 0,
                },
            });
        }
        if let Some(keyword) = order_by_keyword(
            &tokens,
            statement_cursor,
            &prefix,
            context,
            projection_complete,
            active_scopes.last().copied().flatten(),
        ) {
            candidates.push(CompletionCandidate {
                label: keyword.to_owned(),
                insert_text: keyword.to_owned(),
                kind: CompletionKind::Keyword,
                detail: None,
                replace,
                score: CompletionScore {
                    context: 4,
                    name_match: 2,
                    schema: 0,
                },
            });
        }
    }
    if qualifiers.is_empty() {
        let keyword_prefix_match =
            matches!(context, Context::Ddl(DdlContext::AlterColumnDefinition))
                && keywords_for_completion(context, dialect, projection_complete, ordering_stage)
                    .iter()
                    .any(|keyword| keyword.to_ascii_lowercase().starts_with(&folded_prefix));
        for data_type in data_types_for_context(context, dialect) {
            if !keyword_prefix_match && data_type.to_ascii_lowercase().starts_with(&folded_prefix) {
                let insert_text = if matches!(
                    context,
                    Context::Ddl(DdlContext::AlterColumnDefinition)
                        | Context::Ddl(DdlContext::AlterColumnType(true))
                ) && dialect == SqlDialect::Postgres
                {
                    format!("TYPE {data_type}")
                } else {
                    (*data_type).to_owned()
                };
                candidates.push(CompletionCandidate {
                    label: (*data_type).to_owned(),
                    insert_text,
                    kind: CompletionKind::DataType,
                    detail: Some("data type".to_owned()),
                    replace,
                    score: CompletionScore {
                        context: 4,
                        name_match: 2,
                        schema: 0,
                    },
                });
            }
        }
    }
    if qualifiers.is_empty()
        && !matches!(
            ordering_stage,
            Some(
                OrderingStage::Direction
                    | OrderingStage::AfterDirection
                    | OrderingStage::NullPlacement
                    | OrderingStage::Complete,
            )
        )
        && !cursor_in_literal_or_quoted(&tokens, statement_cursor)
    {
        match context {
            Context::Expression(_) => push_builtin_candidates(
                &mut candidates,
                expression_builtins(dialect),
                &prefix,
                replace,
                2,
            ),
            Context::Ddl(DdlContext::DefaultValue) => push_builtin_candidates(
                &mut candidates,
                default_value_builtins(dialect),
                &prefix,
                replace,
                3,
            ),
            _ => {}
        }
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.label.cmp(&right.label))
    });
    candidates
}

pub fn completion_dependencies(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> CompletionDependencies {
    let cursor = cursor.min(text.len());
    let (replace, prefix, _, _) = identifier_at(text, cursor, dialect);
    let (statement, statement_cursor) = current_statement(text, replace.start, dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    let context = context_at(
        &tokens,
        statement_cursor,
        active_scopes.last().copied().flatten(),
        dialect,
        &prefix,
    );
    let mut relation_children = HashSet::new();
    if context == Context::AssignmentTarget {
        relation_children.extend(assignment_target_ids(
            &tokens,
            statement_cursor,
            dialect,
            index,
            completion_context,
        ));
    } else if let Some((relation, _)) = ddl_child_parent(
        &tokens,
        statement_cursor,
        context,
        index,
        completion_context,
    ) {
        relation_children.insert(relation);
    } else if let Some(relation) = relation_after_keyword(
        &tokens,
        statement_cursor,
        "references",
        index,
        completion_context,
    ) {
        relation_children.insert(relation);
    } else {
        visible_relation_bindings(&tokens, &active_scopes)
            .into_iter()
            .flat_map(|binding| relation_ids(index, &binding, completion_context))
            .for_each(|relation| {
                relation_children.insert(relation);
            });
    }
    CompletionDependencies {
        relation_children: relation_children.into_iter().collect(),
    }
}

fn relation_after_keyword(
    tokens: &[CompletionToken],
    cursor: usize,
    keyword: &str,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Option<CatalogId> {
    let position = tokens.iter().position(|token| {
        token.end <= cursor
            && !token.quoted
            && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case(keyword))
    })?;
    let name = token_word(Some(tokens.get(position + 1)?))?;
    relation_ids(
        index,
        &RelationBinding {
            name: name.split('.').map(str::to_owned).collect(),
            alias: None,
            depth: 0,
            scope_start: None,
        },
        completion_context,
    )
    .into_iter()
    .next()
}

pub fn qualifier_segments_at(text: &str, cursor: usize, dialect: SqlDialect) -> Vec<String> {
    let (_, _, qualifiers, _) = identifier_at(text, cursor, dialect);
    qualifiers
}

pub fn relation_ids_for_completion(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Vec<CatalogId> {
    completion_dependencies(text, cursor, dialect, index, completion_context).relation_children
}

fn completion_detail(entry: &CatalogEntry) -> Option<String> {
    match &entry.metadata {
        CatalogMetadata::Column(column) => Some(super::short_type_name(&column.native_type)),
        CatalogMetadata::None | CatalogMetadata::Index(_) | CatalogMetadata::Constraint(_) => None,
    }
}

fn relation_detail(entry: &CatalogEntry) -> Option<String> {
    let mut parts = [
        entry.qualified_name.database.as_deref(),
        entry.qualified_name.schema.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(display_text)
    .collect::<Vec<_>>();
    parts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    (!parts.is_empty()).then(|| format!("({})", parts.join(".")))
}

fn schema_detail(entry: &CatalogEntry) -> Option<String> {
    entry
        .qualified_name
        .database
        .as_ref()
        .map(|database| format!("({})", display_text(database)))
}

fn relation_insert_text(
    entry: &CatalogEntry,
    context: CompletionContext<'_>,
    dialect: SqlDialect,
    qualifiers: &[String],
) -> String {
    let object = entry.qualified_name.object.as_str();
    if !qualifiers.is_empty() {
        return quote_identifier(object, dialect);
    }
    let database = entry.qualified_name.database.as_deref();
    let schema = entry.qualified_name.schema.as_deref();
    let parts = match dialect {
        SqlDialect::MySql | SqlDialect::MariaDb => {
            if database.is_some_and(|value| context.database == Some(value)) {
                vec![object]
            } else {
                vec![database.unwrap_or_default(), object]
            }
        }
        SqlDialect::Sqlite | SqlDialect::DuckDb => {
            if schema.is_some_and(|value| context.schema == Some(value)) {
                vec![object]
            } else {
                vec![schema.or(database).unwrap_or_default(), object]
            }
        }
        SqlDialect::Snowflake => {
            if database == context.database {
                if schema == context.schema {
                    vec![object]
                } else {
                    vec![schema.unwrap_or_default(), object]
                }
            } else {
                vec![
                    database.unwrap_or_default(),
                    schema.unwrap_or_default(),
                    object,
                ]
            }
        }
        SqlDialect::Postgres | SqlDialect::SqlServer | SqlDialect::Generic | SqlDialect::Oracle => {
            if database.is_some_and(|value| {
                context
                    .database
                    .is_some_and(|active| active.eq_ignore_ascii_case(value))
            }) {
                if schema.is_some_and(|value| {
                    context
                        .schema
                        .is_some_and(|active| active.eq_ignore_ascii_case(value))
                }) {
                    vec![object]
                } else {
                    vec![schema.unwrap_or_default(), object]
                }
            } else {
                vec![
                    database.unwrap_or_default(),
                    schema.unwrap_or_default(),
                    object,
                ]
            }
        }
        SqlDialect::ClickHouse => vec![object],
    };
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| quote_relation_component(part, dialect))
        .collect::<Vec<_>>()
        .join(".")
}

fn quote_relation_component(value: &str, dialect: SqlDialect) -> String {
    if dialect == SqlDialect::Snowflake {
        return quote_identifier(value, dialect);
    }
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
    {
        value.to_owned()
    } else {
        quote_identifier(value, dialect)
    }
}

fn column_insert_text(value: &str, dialect: SqlDialect, quoted: bool) -> String {
    if matches!(dialect, SqlDialect::SqlServer | SqlDialect::Snowflake)
        || quoted
        || !is_bare_identifier(value)
        || is_reserved_word(value, dialect)
    {
        quote_identifier(value, dialect)
    } else {
        value.to_owned()
    }
}

fn is_bare_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_reserved_word(value: &str, dialect: SqlDialect) -> bool {
    let word = value.to_ascii_uppercase();
    let common = [
        "SELECT",
        "FROM",
        "WHERE",
        "TABLE",
        "ALTER",
        "DROP",
        "CREATE",
        "INSERT",
        "UPDATE",
        "DELETE",
        "NULL",
        "DEFAULT",
        "PRIMARY",
        "KEY",
        "UNIQUE",
        "CHECK",
        "REFERENCES",
    ];
    common.contains(&word.as_str())
        || matches!(
            (dialect, word.as_str()),
            (SqlDialect::MySql, "MODIFY" | "CHANGE" | "INDEX")
                | (SqlDialect::Postgres, "RETURNING" | "ILIKE")
                | (SqlDialect::SqlServer, "TOP" | "IDENTITY")
        )
}

pub fn should_offer_completion(text: &str, cursor: usize) -> bool {
    should_offer_completion_for_dialect(text, cursor, SqlDialect::Generic)
}

pub fn should_offer_completion_for_dialect(text: &str, cursor: usize, dialect: SqlDialect) -> bool {
    let cursor = cursor.min(text.len());
    if cursor == 0 || cursor_is_in_comment_or_literal(&text[..cursor], dialect) {
        return false;
    }
    if text[..cursor]
        .rsplit('\n')
        .next()
        .is_none_or(|line| line.trim().is_empty())
    {
        return false;
    }
    let bytes = text.as_bytes();
    let Some(previous) = cursor.checked_sub(1).and_then(|index| bytes.get(index)) else {
        return false;
    };
    if previous.is_ascii_alphanumeric() || *previous == b'_' || *previous >= 0x80 {
        return true;
    }
    if *previous != b'.' || cursor < 2 {
        return should_offer_alter_table_completion(text, cursor, dialect)
            || should_offer_ordering_completion(text, cursor, dialect);
    }
    let mut qualifier_start = cursor - 2;
    while qualifier_start > 0 && is_identifier_byte(bytes[qualifier_start - 1], dialect) {
        qualifier_start -= 1;
    }
    !bytes[qualifier_start].is_ascii_digit()
}

fn should_offer_alter_table_completion(text: &str, cursor: usize, dialect: SqlDialect) -> bool {
    let (statement, statement_cursor) = current_statement(text, cursor.min(text.len()), dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    let context = context_at(
        &tokens,
        statement_cursor,
        active_scopes.last().copied().flatten(),
        dialect,
        "",
    );
    matches!(
        context,
        Context::Ddl(
            DdlContext::AlterTableAction
                | DdlContext::ExistingColumn
                | DdlContext::ExistingConstraint
                | DdlContext::ExistingIndex
                | DdlContext::AlterColumnDefinition
                | DdlContext::AlterColumnType(_)
                | DdlContext::DefaultValue
        )
    )
}

fn should_offer_ordering_completion(text: &str, cursor: usize, dialect: SqlDialect) -> bool {
    let (statement, statement_cursor) = current_statement(text, cursor.min(text.len()), dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    let current_scope = active_scopes.last().copied().flatten();
    let context = context_at(&tokens, statement_cursor, current_scope, dialect, "");
    if !matches!(context, Context::Expression(ExpressionContext::Ordering)) {
        return false;
    }
    let Some(last) = tokens
        .iter()
        .rev()
        .find(|token| token.end <= statement_cursor)
    else {
        return false;
    };
    matches!(
        last.kind,
        CompletionTokenKind::Comma
            | CompletionTokenKind::Word(_)
            | CompletionTokenKind::RightParen
            | CompletionTokenKind::Literal
            | CompletionTokenKind::Star
    )
}

fn cursor_is_in_comment_or_literal(text: &str, dialect: SqlDialect) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'-' && bytes.get(index + 1) == Some(&b'-') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            if index == bytes.len() {
                return true;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            if index + 1 == bytes.len() {
                return true;
            }
            index += 2;
            continue;
        }
        if bytes[index] == b'\'' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                    } else {
                        index += 1;
                        break;
                    }
                } else {
                    index += 1;
                }
            }
            if index == bytes.len() && bytes.last() != Some(&b'\'') {
                return true;
            }
            continue;
        }
        let quote = match bytes[index] {
            b'"' if !matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse) => Some(b'"'),
            b'[' if dialect == SqlDialect::SqlServer => Some(b']'),
            b'`' if matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse) => Some(b'`'),
            _ => None,
        };
        if let Some(quote) = quote {
            index += 1;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            if index == bytes.len() {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn candidate_indices(
    index: &CompletionIndex,
    parent: Option<&CatalogId>,
    prefix: &str,
) -> Vec<usize> {
    if let Some(parent) = parent {
        return index.children.get(parent).cloned().unwrap_or_default();
    }
    let mut seen = HashSet::new();
    let mut candidates = prefixed_indices(&index.by_name, prefix).collect::<Vec<_>>();
    if !prefix.is_empty() {
        candidates.extend(prefixed_indices(
            &index.by_compact_name,
            &compact_identifier(prefix),
        ));
    }
    candidates.retain(|position| seen.insert(*position));
    candidates
}

fn prefixed_indices<'a>(
    names: &'a BTreeMap<String, Vec<usize>>,
    prefix: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    names
        .range(prefix.to_owned()..)
        .take_while(move |(name, _)| name.starts_with(prefix))
        .flat_map(|(_, values)| values.iter().copied())
}

fn completion_kind(kind: CatalogKind) -> Option<CompletionKind> {
    Some(match kind {
        CatalogKind::Database => CompletionKind::Database,
        CatalogKind::Schema => CompletionKind::Schema,
        CatalogKind::Table => CompletionKind::Table,
        CatalogKind::View | CatalogKind::MaterializedView => CompletionKind::View,
        CatalogKind::Column => CompletionKind::Column,
        CatalogKind::Index => CompletionKind::Index,
        CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint => CompletionKind::Constraint,
        CatalogKind::Function => CompletionKind::Function,
        CatalogKind::Procedure => CompletionKind::Procedure,
        CatalogKind::Trigger => CompletionKind::Trigger,
        CatalogKind::Sequence => CompletionKind::Sequence,
        CatalogKind::Type => CompletionKind::Type,
        CatalogKind::Dictionary => CompletionKind::Table,
        CatalogKind::Projection => CompletionKind::Index,
        CatalogKind::UserDefinedFunction => CompletionKind::Function,
    })
}

fn catalog_kind_allowed(context: Context, kind: CompletionKind) -> bool {
    match context {
        Context::Statement | Context::Insert | Context::UpdateSet => false,
        Context::AssignmentTarget => kind == CompletionKind::Column,
        Context::Delete(_) => false,
        Context::Relation => matches!(
            kind,
            CompletionKind::Database
                | CompletionKind::Schema
                | CompletionKind::Table
                | CompletionKind::View
        ),
        Context::Expression(_) => {
            matches!(kind, CompletionKind::Column | CompletionKind::Function)
        }
        Context::Qualifier => matches!(
            kind,
            CompletionKind::Column | CompletionKind::Table | CompletionKind::View
        ),
        Context::Routine => {
            matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        }
        Context::Ddl(ddl) => match ddl {
            DdlContext::ExistingObject(target) => matches!(
                (target, kind),
                (DdlObjectTarget::Database, CompletionKind::Database)
                    | (DdlObjectTarget::Schema, CompletionKind::Schema)
                    | (DdlObjectTarget::Table, CompletionKind::Table)
                    | (DdlObjectTarget::View, CompletionKind::View)
                    | (DdlObjectTarget::Index, CompletionKind::Index)
                    | (DdlObjectTarget::Trigger, CompletionKind::Trigger)
                    | (DdlObjectTarget::Sequence, CompletionKind::Sequence)
                    | (DdlObjectTarget::Type, CompletionKind::Type)
                    | (DdlObjectTarget::Function, CompletionKind::Function)
                    | (DdlObjectTarget::Procedure, CompletionKind::Procedure)
            ),
            DdlContext::CreateIndexTarget => matches!(
                kind,
                CompletionKind::Database
                    | CompletionKind::Schema
                    | CompletionKind::Table
                    | CompletionKind::View
            ),
            DdlContext::ExistingColumn => kind == CompletionKind::Column,
            DdlContext::ExistingConstraint => kind == CompletionKind::Constraint,
            DdlContext::ExistingIndex => kind == CompletionKind::Index,
            DdlContext::ReferenceRelation => kind == CompletionKind::Table,
            DdlContext::ReferenceColumn => kind == CompletionKind::Column,
            DdlContext::NewColumnName => false,
            DdlContext::RenameColumnTo | DdlContext::NewObjectName => false,
            _ => false,
        },
    }
}

#[derive(Clone, Copy, Debug)]
struct TableElement {
    start: usize,
    default_active: bool,
}

fn create_table_element(tokens: &[CompletionToken], cursor: usize) -> Option<TableElement> {
    let table_paren = tokens.iter().find(|token| {
        token.kind == CompletionTokenKind::LeftParen && token.depth == 0 && token.end <= cursor
    })?;
    let element_level = table_paren.depth + 1;
    let mut start = table_paren.end;
    let mut default_start: Option<usize> = None;
    for token in tokens.iter().filter(|token| {
        token.start >= table_paren.end && token.end <= cursor && token.depth >= element_level
    }) {
        match &token.kind {
            CompletionTokenKind::Comma if token.depth == element_level => {
                if default_start.is_some() {
                    default_start = None;
                }
                start = token.end;
            }
            CompletionTokenKind::RightParen if token.depth == table_paren.depth => return None,
            CompletionTokenKind::Word(word) if !token.quoted && token.depth == element_level => {
                if default_start.is_some() && is_default_end_keyword(word) {
                    default_start = None;
                } else if word.eq_ignore_ascii_case("default") && default_start.is_none() {
                    default_start = Some(token.start);
                }
            }
            _ => {}
        }
    }
    Some(TableElement {
        start,
        default_active: default_start.is_some(),
    })
}

fn is_default_end_keyword(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "not" | "primary" | "unique" | "references" | "check"
    )
}

fn alter_table_default_active(words: &[String]) -> bool {
    let Some(index) = words.iter().rposition(|word| word == "default") else {
        return false;
    };
    let action = words.get(3).map(String::as_str);
    let is_add = action == Some("add");
    let is_set = match action {
        Some("alter") => words
            .get(index.saturating_sub(1))
            .is_some_and(|word| word == "set"),
        _ => false,
    };
    if !(is_add || is_set) {
        return false;
    }
    if is_add && words.get(4).is_some_and(|word| word == "constraint") {
        return false;
    }
    words[index + 1..].iter().all(|word| {
        !is_default_end_keyword(word)
            && !matches!(word.as_str(), "add" | "alter" | "drop" | "rename")
    })
}

fn cursor_in_literal_or_quoted(tokens: &[CompletionToken], cursor: usize) -> bool {
    tokens.iter().any(|token| {
        (token.kind == CompletionTokenKind::Literal || token.quoted)
            && token.start < cursor
            && cursor <= token.end
    })
}

fn push_builtin_candidates(
    candidates: &mut Vec<CompletionCandidate>,
    builtins: impl IntoIterator<Item = Builtin>,
    prefix: &str,
    replace: TextRange,
    context_score: u8,
) {
    for builtin in builtins {
        if prefix.is_empty() && builtin.kind != CompletionKind::BuiltinExpression {
            continue;
        }
        let Some(name_match) = identifier_match(builtin.name, prefix) else {
            continue;
        };
        candidates.push(CompletionCandidate {
            label: builtin.name.to_owned(),
            insert_text: builtin.name.to_owned(),
            kind: builtin.kind,
            detail: Some(builtin.detail.to_owned()),
            replace,
            score: CompletionScore {
                context: context_score,
                name_match: match name_match {
                    IdentifierMatch::CompactPrefix => 1,
                    IdentifierMatch::Prefix => 2,
                    IdentifierMatch::Exact => 3,
                },
                schema: 0,
            },
        });
    }
}

fn assignment_context(
    tokens: &[CompletionToken],
    cursor: usize,
    dialect: SqlDialect,
) -> Option<(Context, RelationBinding)> {
    // Parentheses inherit assignment expressions, but a nested query owns its context.
    let scopes = active_scope_starts(tokens, cursor);
    let command = scopes.iter().rev().find_map(|scope| {
        tokens.iter().find(|token| {
            token.end <= cursor
                && token.scope_start == *scope
                && !token.quoted
                && token_word(Some(token)).is_some_and(|word| {
                    matches!(
                        word.to_ascii_lowercase().as_str(),
                        "select"
                            | "update"
                            | "insert"
                            | "delete"
                            | "create"
                            | "alter"
                            | "drop"
                            | "set"
                            | "truncate"
                            | "merge"
                    )
                })
        })
    })?;
    if !token_word(Some(command))?.eq_ignore_ascii_case("update") {
        return None;
    }
    let mut context = None;
    let mut brackets: usize = 0;
    for token in tokens.iter().filter(|token| {
        token.start > command.start
            && token.end <= cursor
            && token.scope_start == command.scope_start
    }) {
        if token.quoted {
            continue;
        }
        match &token.kind {
            CompletionTokenKind::LeftBracket => brackets += 1,
            CompletionTokenKind::RightBracket => brackets = brackets.saturating_sub(1),
            CompletionTokenKind::Word(word)
                if word.eq_ignore_ascii_case("set") && context.is_none() =>
            {
                context = Some(Context::AssignmentTarget);
            }
            CompletionTokenKind::Word(word)
                if matches!(
                    word.to_ascii_lowercase().as_str(),
                    "where" | "from" | "returning" | "order" | "limit"
                ) =>
            {
                return None;
            }
            CompletionTokenKind::Word(word)
                if dialect == SqlDialect::SqlServer
                    && context == Some(Context::Expression(ExpressionContext::AssignmentValue))
                    && word.eq_ignore_ascii_case("output") =>
            {
                return None;
            }
            CompletionTokenKind::Equals if context == Some(Context::AssignmentTarget) => {
                context = Some(Context::Expression(ExpressionContext::AssignmentValue));
            }
            CompletionTokenKind::Comma if context.is_some() && brackets == 0 => {
                context = Some(Context::AssignmentTarget);
            }
            _ => {}
        }
    }
    let position = tokens
        .iter()
        .position(|token| token.start == command.start)?;
    if context.is_none() {
        // Exclude the word being completed and any SQL after the cursor from alias parsing.
        let before_cursor = &tokens[..tokens.partition_point(|token| token.end <= cursor)];
        let (target, end) = relation_binding_at(before_cursor, position + 1)?;
        return (end == before_cursor.len()).then_some((Context::UpdateSet, target));
    }
    let (target, _) = relation_binding_at(tokens, position + 1)?;
    context.map(|context| (context, target))
}

fn assignment_target_ids(
    tokens: &[CompletionToken],
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Vec<CatalogId> {
    let Some((_, target)) = assignment_context(tokens, cursor, dialect) else {
        return Vec::new();
    };
    if dialect == SqlDialect::SqlServer && target.name.len() == 1 {
        // UPDATE alias resolves through same-query FROM/JOIN bindings before catalog names.
        let sources = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| {
                !token.quoted
                    && token.scope_start == target.scope_start
                    && token_word(Some(token)).is_some_and(|word| {
                        word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("join")
                    })
            })
            .filter_map(|(position, _)| {
                relation_binding_at(tokens, position + 1).map(|(binding, _)| binding)
            })
            .filter(|binding| {
                binding
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(&target.name[0]))
            })
            .collect::<Vec<_>>();
        if !sources.is_empty() {
            return sources
                .iter()
                .flat_map(|source| relation_ids(index, source, completion_context))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
        }
    }
    relation_ids(index, &target, completion_context)
}

fn relation_child_candidate_indices(
    index: &CompletionIndex,
    relations: &[CatalogId],
) -> Vec<usize> {
    relations
        .iter()
        .flat_map(|relation| index.children.get(relation).into_iter().flatten().copied())
        .collect()
}

fn context_at(
    tokens: &[CompletionToken],
    cursor: usize,
    current_scope: Option<usize>,
    dialect: SqlDialect,
    prefix: &str,
) -> Context {
    if let Some((context, _)) = assignment_context(tokens, cursor, dialect) {
        return context;
    }
    let is_ddl = tokens.iter().any(|token| {
        token.start < cursor
            && !token.quoted
            && token_word(Some(token)).is_some_and(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "create" | "alter" | "drop" | "truncate"
                )
            })
    });
    let raw_tokens = tokens;
    let tokens = raw_tokens
        .iter()
        .filter(|token| {
            token.end <= cursor
                && (is_ddl
                    && !raw_tokens.iter().any(|query_token| {
                        query_token.start < cursor
                            && !query_token.quoted
                            && token_word(Some(query_token))
                                .is_some_and(|word| word.eq_ignore_ascii_case("select"))
                            && query_token.start > token.start
                    })
                    || token.scope_start == current_scope
                    || (current_scope.is_some()
                        && token.scope_start.is_none()
                        && token.start < current_scope.unwrap_or_default()))
        })
        .collect::<Vec<_>>();
    let mut context = Context::Statement;
    for (index, token) in tokens.iter().enumerate() {
        let CompletionTokenKind::Word(word) = &token.kind else {
            continue;
        };
        if token.quoted {
            continue;
        }
        context = match word.to_ascii_lowercase().as_str() {
            "insert" => Context::Insert,
            "from" | "join" | "update" | "into" => Context::Relation,
            "select" => Context::Expression(ExpressionContext::Projection),
            "where" | "on" | "having" => Context::Expression(ExpressionContext::Predicate),
            "returning" => Context::Expression(ExpressionContext::Returning),
            "group"
                if token_word(tokens.get(index + 1).copied())
                    .is_some_and(|word| word.eq_ignore_ascii_case("by")) =>
            {
                Context::Expression(ExpressionContext::Grouping)
            }
            "order"
                if token_word(tokens.get(index + 1).copied())
                    .is_some_and(|word| word.eq_ignore_ascii_case("by")) =>
            {
                Context::Expression(ExpressionContext::Ordering)
            }
            "call" | "execute" => Context::Routine,
            "create" => Context::Ddl(DdlContext::CreateObjectKind),
            "alter" => Context::Ddl(DdlContext::AlterObjectKind),
            "drop" => Context::Ddl(DdlContext::DropObjectKind),
            "truncate" => Context::Ddl(DdlContext::ExistingObject(DdlObjectTarget::Table)),
            _ => context,
        };
    }
    let words = tokens
        .iter()
        .filter(|token| !token.quoted)
        .filter_map(|token| token_word(Some(*token)))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if let Some(references) = words.iter().position(|word| word == "references")
        && words.len() > references + 1
    {
        return if tokens.iter().any(|token| {
            token.start < cursor
                && token.kind == CompletionTokenKind::LeftParen
                && token.start
                    > tokens
                        .iter()
                        .filter(|token| !token.quoted && token_word(Some(token)).is_some())
                        .nth(references)
                        .map_or(0, |token| token.start)
        }) {
            Context::Ddl(DdlContext::ReferenceColumn)
        } else {
            Context::Ddl(DdlContext::ReferenceRelation)
        };
    }
    if let Some(delete_context) = delete_context(&tokens) {
        return delete_context;
    }
    if words.first().map(String::as_str) == Some("create")
        && words.iter().any(|word| word == "index")
        && let Some(on) = words.iter().position(|word| word == "on")
        && words.len() > on + 1
        && tokens.iter().any(|token| {
            token.start < cursor
                && token.kind == CompletionTokenKind::LeftParen
                && token.start
                    > tokens
                        .iter()
                        .filter(|token| !token.quoted && token_word(Some(token)).is_some())
                        .nth(on)
                        .map_or(0, |token| token.start)
        })
    {
        return Context::Ddl(DdlContext::ReferenceColumn);
    }
    if let Some(first) = words.first().map(String::as_str)
        && matches!(first, "create" | "alter" | "drop" | "truncate")
    {
        context = ddl_context_from_words(raw_tokens, &words, context, dialect, prefix, cursor);
    }
    if words.first().map(String::as_str) == Some("alter") && alter_table_default_active(&words) {
        context = Context::Ddl(DdlContext::DefaultValue);
    }
    if matches!(
        context,
        Context::Ddl(DdlContext::ExistingObject(DdlObjectTarget::Table))
    ) && words.first().map(String::as_str) == Some("create")
        && let Some(element) = create_table_element(raw_tokens, cursor)
    {
        context = if element.default_active {
            Context::Ddl(DdlContext::DefaultValue)
        } else {
            let element_words = raw_tokens
                .iter()
                .filter(|token| {
                    token.start >= element.start
                        && token.end <= cursor
                        && token.depth >= 1
                        && !token.quoted
                })
                .filter_map(|token| token_word(Some(token)))
                .filter(|word| !(*word).eq_ignore_ascii_case("create"))
                .count();
            let has_type = raw_tokens.iter().any(|token| {
                token.start >= element.start
                    && token.end <= cursor
                    && token.depth >= 1
                    && !token.quoted
                    && token_word(Some(token)).is_some_and(|word| {
                        matches!(
                            word.to_ascii_uppercase().as_str(),
                            "INT"
                                | "INTEGER"
                                | "BIGINT"
                                | "TEXT"
                                | "VARCHAR"
                                | "NUMERIC"
                                | "DECIMAL"
                                | "BOOLEAN"
                                | "JSON"
                                | "JSONB"
                                | "DATE"
                                | "TIMESTAMP"
                                | "DATETIME"
                                | "DATETIME2"
                                | "REAL"
                                | "BLOB"
                                | "BIT"
                                | "NVARCHAR"
                                | "UNIQUEIDENTIFIER"
                        )
                    })
            });
            if element_words == 0 {
                Context::Ddl(DdlContext::TableConstraint)
            } else if element_words <= 1 || !has_type {
                Context::Ddl(DdlContext::ColumnType)
            } else {
                Context::Ddl(DdlContext::ColumnConstraint)
            }
        };
    }
    if matches!(
        tokens.last().map(|token| &token.kind),
        Some(CompletionTokenKind::Dot)
    ) && context != Context::Relation
        && !matches!(context, Context::Ddl(_))
    {
        Context::Qualifier
    } else {
        context
    }
}

fn delete_context(tokens: &[&CompletionToken]) -> Option<Context> {
    let first_word = tokens.iter().find_map(|token| {
        (!token.quoted).then(|| token_word(Some(*token)).map(str::to_ascii_lowercase))?
    });
    if !matches!(first_word.as_deref(), Some("delete" | "with")) {
        return None;
    }
    let command = tokens.iter().enumerate().rev().find_map(|(index, token)| {
        if token.quoted {
            return None;
        }
        let word = token_word(Some(*token))?;
        let lower = word.to_ascii_lowercase();
        if !matches!(lower.as_str(), "delete" | "select") {
            return None;
        }
        let previous = tokens.get(index.wrapping_sub(1)).copied();
        let command_position = previous.is_none()
            || matches!(
                previous.map(|token| &token.kind),
                Some(CompletionTokenKind::LeftParen | CompletionTokenKind::RightParen)
            );
        if !command_position {
            return None;
        }
        Some((index, word))
    })?;
    if command.1.eq_ignore_ascii_case("select") {
        return None;
    }
    let delete = command.0;
    let Some(from_token) = tokens.get(delete + 1).copied() else {
        return Some(Context::Delete(DeleteContext::From));
    };
    let from_word = (!from_token.quoted)
        .then(|| token_word(Some(from_token)))
        .flatten()
        .map(str::to_ascii_lowercase);
    if from_word.as_deref() != Some("from") {
        return if delete + 2 == tokens.len()
            && from_word.is_some_and(|word| "from".starts_with(&word))
        {
            Some(Context::Delete(DeleteContext::From))
        } else {
            None
        };
    }
    let from = delete + 1;
    let first = tokens.get(from + 1)?;
    token_word(Some(*first))?;
    let mut target_end = from + 2;
    while tokens
        .get(target_end)
        .is_some_and(|token| matches!(token.kind, CompletionTokenKind::Dot))
    {
        tokens
            .get(target_end + 1)
            .and_then(|token| token_word(Some(*token)))?;
        target_end += 2;
    }
    let tail = &tokens[target_end..];
    if tail
        .iter()
        .any(|token| !token.quoted && token_word(Some(*token)).is_some_and(is_delete_clause_word))
    {
        return None;
    }
    let first = match tail.first() {
        Some(token) => token_word(Some(*token)),
        None => return Some(Context::Delete(DeleteContext::AfterTarget)),
    };
    let word = first?;
    if word.eq_ignore_ascii_case("as") {
        return if tail.len() == 1 {
            Some(Context::Delete(DeleteContext::Alias))
        } else {
            Some(Context::Delete(DeleteContext::AfterTarget))
        };
    }
    Some(Context::Delete(DeleteContext::AfterTarget))
}

fn is_delete_clause_word(word: &str) -> bool {
    is_relation_boundary(word) || matches!(word.to_ascii_lowercase().as_str(), "from" | "using")
}

fn order_by_keyword(
    tokens: &[CompletionToken],
    cursor: usize,
    prefix: &str,
    context: Context,
    projection_complete: bool,
    current_scope: Option<usize>,
) -> Option<&'static str> {
    if !(matches!(
        context,
        Context::Relation
            | Context::Expression(ExpressionContext::Predicate | ExpressionContext::Grouping,)
    ) || projection_complete && context == Context::Expression(ExpressionContext::Projection))
    {
        return None;
    }

    let current_start = tokens
        .iter()
        .find(|token| token.start < cursor && token.end >= cursor)
        .map_or(cursor, |token| token.start);
    let previous = tokens
        .iter()
        .rfind(|token| token.end <= current_start && token.scope_start == current_scope);

    if previous.is_some_and(|token| {
        token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("order"))
    }) {
        return "BY"
            .starts_with(&prefix.to_ascii_uppercase())
            .then_some("BY");
    }

    if prefix.is_empty() {
        return None;
    }
    if !"order".starts_with(&prefix.to_ascii_lowercase()) {
        return None;
    }
    if previous.is_none_or(|token| {
        matches!(
            token.kind,
            CompletionTokenKind::Operator
                | CompletionTokenKind::Equals
                | CompletionTokenKind::Comma
                | CompletionTokenKind::LeftParen
        ) || token_word(Some(token)).is_some_and(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "where" | "and" | "or" | "not" | "between" | "is" | "like" | "in"
            )
        })
    }) {
        return None;
    }
    Some("ORDER BY")
}

fn keywords_for_completion(
    context: Context,
    dialect: SqlDialect,
    projection_complete: bool,
    ordering_stage: Option<OrderingStage>,
) -> &'static [&'static str] {
    if let Some(stage) = ordering_stage {
        return match stage {
            OrderingStage::Expression => &[],
            OrderingStage::Direction => match dialect {
                SqlDialect::Postgres
                | SqlDialect::Sqlite
                | SqlDialect::DuckDb
                | SqlDialect::Generic
                | SqlDialect::Oracle
                | SqlDialect::Snowflake => &["ASC", "DESC", "NULLS FIRST", "NULLS LAST"],
                SqlDialect::ClickHouse => &["ASC", "DESC"],
                SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::SqlServer => &["ASC", "DESC"],
            },
            OrderingStage::AfterDirection => match dialect {
                SqlDialect::Postgres
                | SqlDialect::Sqlite
                | SqlDialect::DuckDb
                | SqlDialect::Generic
                | SqlDialect::Oracle
                | SqlDialect::Snowflake => &["NULLS FIRST", "NULLS LAST"],
                SqlDialect::ClickHouse => &[],
                SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::SqlServer => &[],
            },
            OrderingStage::NullPlacement => match dialect {
                SqlDialect::Postgres
                | SqlDialect::Sqlite
                | SqlDialect::DuckDb
                | SqlDialect::Generic
                | SqlDialect::Oracle
                | SqlDialect::Snowflake => &["FIRST", "LAST"],
                SqlDialect::ClickHouse => &[],
                SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::SqlServer => &[],
            },
            OrderingStage::Complete => &[],
        };
    }
    keywords(context, dialect, projection_complete)
}

fn ordering_stage(
    tokens: &[CompletionToken],
    cursor: usize,
    current_scope: Option<usize>,
) -> OrderingStage {
    let tokens = tokens
        .iter()
        .filter(|token| token.end <= cursor && token.scope_start == current_scope)
        .collect::<Vec<_>>();
    let Some(order_index) = tokens.iter().rposition(|token| {
        token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("order"))
    }) else {
        return OrderingStage::Expression;
    };
    let Some(by_index) = tokens.iter().position(|token| {
        token.start > tokens[order_index].end
            && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("by"))
    }) else {
        return OrderingStage::Expression;
    };
    let item = &tokens[by_index + 1..];
    let Some(last) = item.last() else {
        return OrderingStage::Expression;
    };
    match token_word(Some(last))
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("asc") | Some("desc") => OrderingStage::AfterDirection,
        Some("nulls") => OrderingStage::NullPlacement,
        Some("first") | Some("last") => OrderingStage::Complete,
        _ if matches!(
            last.kind,
            CompletionTokenKind::Comma
                | CompletionTokenKind::Operator
                | CompletionTokenKind::Equals
        ) =>
        {
            OrderingStage::Expression
        }
        _ => OrderingStage::Direction,
    }
}

fn ddl_child_parent(
    tokens: &[CompletionToken],
    cursor: usize,
    context: Context,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Option<(CatalogId, CompletionKind)> {
    let words = tokens
        .iter()
        .filter(|token| token.end <= cursor && !token.quoted)
        .filter_map(|token| token_word(Some(token)))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let position = match context {
        Context::Ddl(DdlContext::ExistingColumn)
        | Context::Ddl(DdlContext::ExistingConstraint)
        | Context::Ddl(DdlContext::ExistingIndex) => Some(2),
        Context::Ddl(DdlContext::ReferenceColumn) => words
            .iter()
            .position(|word| word.eq_ignore_ascii_case("references"))
            .or_else(|| {
                words
                    .iter()
                    .position(|word| word.eq_ignore_ascii_case("on"))
            })
            .map(|position| position + 1),
        Context::Ddl(DdlContext::CreateIndexTarget) => words
            .iter()
            .position(|word| word.eq_ignore_ascii_case("on"))
            .map(|position| position + 1),
        _ => None,
    }?;
    let name = words.get(position)?;
    let binding = RelationBinding {
        name: name.split('.').map(str::to_owned).collect(),
        alias: None,
        depth: 0,
        scope_start: None,
    };
    let relation = relation_ids(index, &binding, completion_context)
        .into_iter()
        .next()?;
    let kind = match context {
        Context::Ddl(DdlContext::ExistingColumn | DdlContext::ReferenceColumn) => {
            CompletionKind::Column
        }
        Context::Ddl(DdlContext::ExistingConstraint) => CompletionKind::Constraint,
        Context::Ddl(DdlContext::ExistingIndex) => CompletionKind::Index,
        Context::Ddl(DdlContext::CreateIndexTarget) => CompletionKind::Column,
        _ => return None,
    };
    Some((relation, kind))
}

fn ddl_context_from_words(
    tokens: &[CompletionToken],
    words: &[String],
    fallback: Context,
    dialect: SqlDialect,
    prefix: &str,
    cursor: usize,
) -> Context {
    let word = |value: &str| words.iter().position(|item| item == value);
    let Some(first) = words.first().map(String::as_str) else {
        return fallback;
    };
    let target = match (first, words.get(1).map(String::as_str)) {
        ("create", Some("table")) => Some(DdlObjectTarget::Table),
        ("create", Some("view")) => Some(DdlObjectTarget::View),
        ("create", Some("schema")) => Some(DdlObjectTarget::Schema),
        ("create", Some("database")) => Some(DdlObjectTarget::Database),
        ("create", Some("index")) => Some(DdlObjectTarget::Index),
        ("create", Some("sequence")) => Some(DdlObjectTarget::Sequence),
        ("create", Some("type")) => Some(DdlObjectTarget::Type),
        ("create", Some("function")) => Some(DdlObjectTarget::Function),
        ("create", Some("procedure")) => Some(DdlObjectTarget::Procedure),
        ("create", Some("trigger")) => Some(DdlObjectTarget::Trigger),
        ("alter", Some("table")) => Some(DdlObjectTarget::Table),
        ("alter", Some("view")) => Some(DdlObjectTarget::View),
        ("alter", Some("schema")) => Some(DdlObjectTarget::Schema),
        ("alter", Some("index")) => Some(DdlObjectTarget::Index),
        ("drop", Some("table")) => Some(DdlObjectTarget::Table),
        ("drop", Some("view")) => Some(DdlObjectTarget::View),
        ("drop", Some("schema")) => Some(DdlObjectTarget::Schema),
        ("drop", Some("database")) => Some(DdlObjectTarget::Database),
        ("drop", Some("index")) => Some(DdlObjectTarget::Index),
        ("drop", Some("trigger")) => Some(DdlObjectTarget::Trigger),
        ("drop", Some("sequence")) => Some(DdlObjectTarget::Sequence),
        ("drop", Some("type")) => Some(DdlObjectTarget::Type),
        ("drop", Some("function")) => Some(DdlObjectTarget::Function),
        ("drop", Some("procedure")) => Some(DdlObjectTarget::Procedure),
        ("truncate", _) => Some(DdlObjectTarget::Table),
        _ => None,
    };
    if first == "create" && word("index").is_some() && word("on").is_some() {
        return Context::Ddl(DdlContext::CreateIndexTarget);
    }
    if first == "drop" && word("index").is_some() && word("on").is_some() {
        return if dialect == SqlDialect::SqlServer {
            Context::Relation
        } else {
            Context::Ddl(DdlContext::ExistingObject(DdlObjectTarget::Index))
        };
    }
    if first == "create"
        && let Some(as_index) = word("as")
    {
        let query_words = words.get(as_index + 1..).unwrap_or_default();
        if query_words.is_empty() && "select".starts_with(&prefix.to_ascii_lowercase()) {
            return Context::Statement;
        }
        if query_words
            .first()
            .is_some_and(|item| "select".starts_with(item) || item == "select")
        {
            return if query_words.first().is_some_and(|item| item == "select") {
                Context::Expression(ExpressionContext::Projection)
            } else {
                Context::Statement
            };
        }
    }
    if let Some(target) = target {
        if target == DdlObjectTarget::Table && first == "alter" {
            if let Some((position, action)) = words.iter().enumerate().skip(2).find(|(_, word)| {
                matches!(
                    word.as_str(),
                    "add" | "alter" | "modify" | "change" | "rename"
                )
            }) {
                let action = action.as_str();
                let column_offset = 2;
                if words.get(position + 1).map(String::as_str) == Some("column") {
                    if matches!(action, "modify" | "change" | "alter") {
                        let name_position = position + column_offset;
                        let action_token = tokens.iter().find(|token| {
                            token_word(Some(token)).is_some_and(|word| {
                                !token.quoted && word.eq_ignore_ascii_case(action)
                            })
                        });
                        let column_token = action_token.and_then(|action_token| {
                            tokens.iter().find(|token| {
                                token.start > action_token.end
                                    && token_word(Some(token)).is_some_and(|word| {
                                        !token.quoted && word.eq_ignore_ascii_case("column")
                                    })
                            })
                        });
                        let name = column_token.and_then(|column_token| {
                            tokens.iter().find(|token| {
                                token.start > column_token.end && token_word(Some(token)).is_some()
                            })
                        });
                        if name.is_none() || name.is_some_and(|name| cursor <= name.end) {
                            return Context::Ddl(DdlContext::ExistingColumn);
                        }
                        let definition = words.get(name_position + 1..).unwrap_or_default();
                        let explicit_type = definition.first().is_some_and(|word| {
                            word == "type"
                                || (word == "set"
                                    && definition.get(1).map(String::as_str) == Some("data")
                                    && definition.get(2).map(String::as_str) == Some("type"))
                        });
                        return if explicit_type {
                            Context::Ddl(DdlContext::AlterColumnType(false))
                        } else if definition.is_empty() {
                            if prefix.is_empty()
                                || is_column_constraint_prefix(prefix)
                                || is_column_action_prefix(prefix)
                            {
                                Context::Ddl(DdlContext::AlterColumnDefinition)
                            } else {
                                Context::Ddl(DdlContext::AlterColumnType(true))
                            }
                        } else if matches!(
                            definition.first().map(String::as_str),
                            Some("set" | "drop")
                        ) {
                            Context::Ddl(DdlContext::AlterColumnDefinition)
                        } else {
                            Context::Ddl(DdlContext::AlterColumnType(true))
                        };
                    }
                    if action == "rename" {
                        return match words.get(position + 2).map(String::as_str) {
                            None => Context::Ddl(DdlContext::ExistingColumn),
                            Some(_)
                                if words.get(position + 3).map(String::as_str) == Some("to") =>
                            {
                                Context::Ddl(DdlContext::NewObjectName)
                            }
                            Some(_) if words.len() == position + 3 && prefix.is_empty() => {
                                Context::Ddl(DdlContext::RenameColumnTo)
                            }
                            Some(_) => Context::Ddl(DdlContext::ExistingColumn),
                        };
                    }
                    let name_position = position + column_offset;
                    if words.len() <= name_position
                        || (words.len() == name_position + 1 && !prefix.is_empty())
                    {
                        return if prefix.is_empty() {
                            Context::Ddl(DdlContext::NewColumnName)
                        } else {
                            Context::Ddl(DdlContext::ColumnType)
                        };
                    }
                    let definition = &words[name_position + 1..];
                    if definition.is_empty() {
                        return Context::Ddl(DdlContext::ColumnType);
                    }
                    if definition
                        .first()
                        .is_some_and(|word| is_column_type_word(word, dialect))
                    {
                        return Context::Ddl(DdlContext::ColumnConstraint);
                    }
                    if definition
                        .iter()
                        .any(|word| is_column_constraint_word(word))
                    {
                        return Context::Ddl(DdlContext::ColumnConstraint);
                    }
                    return Context::Ddl(DdlContext::ColumnType);
                }
            }
            let action_position =
                words.iter().enumerate().skip(3).find(|(_, word)| {
                    matches!(word.as_str(), "drop" | "alter" | "modify" | "change")
                });
            if action_position.is_some_and(|(position, _)| {
                words.get(position + 1).map(String::as_str) == Some("column")
            }) {
                return Context::Ddl(DdlContext::ExistingColumn);
            }
            if words.iter().enumerate().skip(3).any(|(position, word)| {
                word == "drop" && words.get(position + 1).map(String::as_str) == Some("constraint")
            }) {
                return Context::Ddl(DdlContext::ExistingConstraint);
            }
            if words.iter().enumerate().skip(3).any(|(position, word)| {
                word == "drop" && words.get(position + 1).map(String::as_str) == Some("index")
            }) {
                return Context::Ddl(DdlContext::ExistingIndex);
            }
            if (words.len() == 3 && prefix.is_empty())
                || (words.len() >= 3 && !prefix.is_empty())
                || words.get(3).is_some_and(|word| {
                    matches!(
                        word.as_str(),
                        "add" | "drop" | "alter" | "rename" | "modify" | "change"
                    )
                })
            {
                return Context::Ddl(DdlContext::AlterTableAction);
            }
        }
        return Context::Ddl(DdlContext::ExistingObject(target));
    }
    match first {
        "create" => Context::Ddl(DdlContext::CreateObjectKind),
        "alter" => Context::Ddl(DdlContext::AlterObjectKind),
        "drop" => Context::Ddl(DdlContext::DropObjectKind),
        _ => fallback,
    }
}

fn is_column_type_word(word: &str, dialect: SqlDialect) -> bool {
    data_types_for_context(Context::Ddl(DdlContext::ColumnType), dialect)
        .iter()
        .any(|data_type| data_type.eq_ignore_ascii_case(word))
}

fn is_column_constraint_word(word: &str) -> bool {
    matches!(
        word,
        "null" | "not" | "default" | "primary" | "unique" | "references" | "check"
    )
}

fn is_column_constraint_prefix(prefix: &str) -> bool {
    [
        "null",
        "not",
        "default",
        "primary",
        "unique",
        "references",
        "check",
    ]
    .iter()
    .any(|keyword| keyword.starts_with(&prefix.to_ascii_lowercase()))
}

fn is_column_action_prefix(prefix: &str) -> bool {
    ["type", "set", "drop"]
        .iter()
        .any(|keyword| keyword.starts_with(&prefix.to_ascii_lowercase()))
}

fn projection_is_complete(
    tokens: &[CompletionToken],
    cursor: usize,
    current_scope: Option<usize>,
) -> bool {
    let tokens = tokens
        .iter()
        .filter(|token| token.end <= cursor && token.scope_start == current_scope)
        .collect::<Vec<_>>();
    let Some(select_index) = tokens.iter().rposition(|token| {
        token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("select"))
    }) else {
        return false;
    };
    let Some(last) = tokens
        .get(select_index + 1..)
        .and_then(|tokens| tokens.last())
    else {
        return false;
    };

    match &last.kind {
        CompletionTokenKind::Literal
        | CompletionTokenKind::Star
        | CompletionTokenKind::RightParen
        | CompletionTokenKind::RightBracket => true,
        CompletionTokenKind::Word(word) => !matches!(
            word.to_ascii_lowercase().as_str(),
            "all"
                | "and"
                | "as"
                | "at"
                | "between"
                | "case"
                | "collate"
                | "distinct"
                | "else"
                | "in"
                | "is"
                | "like"
                | "not"
                | "or"
                | "then"
                | "when"
        ),
        CompletionTokenKind::Operator
        | CompletionTokenKind::Equals
        | CompletionTokenKind::Dot
        | CompletionTokenKind::Comma
        | CompletionTokenKind::LeftParen
        | CompletionTokenKind::LeftBracket => false,
    }
}

fn identifier_at(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
) -> (TextRange, String, Vec<String>, bool) {
    let tokens = completion_tokens(text, dialect);
    let segment = tokens.iter().enumerate().find_map(|(index, token)| {
        (matches!(token.kind, CompletionTokenKind::Word(_))
            && token.start < cursor
            && cursor <= token.end)
            .then_some((index, token))
    });
    if let Some((index, token)) = segment {
        let prefix = text[token.start..cursor]
            .trim_matches(['"', '`', '[', ']'])
            .to_owned();
        let qualifiers = qualifiers_before(&tokens, index);
        return (
            TextRange::new(token.start, token.end),
            prefix,
            qualifiers,
            token.quoted,
        );
    }
    let after_dot = tokens.iter().enumerate().find_map(|(index, token)| {
        (token.kind == CompletionTokenKind::Dot && cursor > token.start && cursor <= token.end)
            .then_some(index)
    });
    if let Some(index) = after_dot {
        return (
            TextRange::new(cursor, cursor),
            String::new(),
            qualifiers_before(&tokens, index + 1),
            false,
        );
    }
    (
        TextRange::new(cursor, cursor),
        String::new(),
        Vec::new(),
        false,
    )
}

fn qualifiers_before(tokens: &[CompletionToken], stop: usize) -> Vec<String> {
    let mut qualifiers = Vec::new();
    let mut index = stop;
    while index >= 2 && tokens[index - 1].kind == CompletionTokenKind::Dot {
        let Some(word) = token_word(tokens.get(index - 2)) else {
            break;
        };
        qualifiers.push(word.to_owned());
        index -= 2;
    }
    qualifiers.reverse();
    qualifiers.retain(|value| !value.is_empty());
    qualifiers
}

fn qualified_candidate_indices(
    index: &CompletionIndex,
    qualifiers: &[String],
    prefix: &str,
    bindings: &[RelationBinding],
    completion_context: CompletionContext<'_>,
    dialect: SqlDialect,
) -> Vec<usize> {
    if qualifiers.is_empty() {
        return candidate_indices(index, None, prefix);
    }
    let qualifier = &qualifiers[0];
    if qualifiers.len() == 1 {
        let alias_parents = bindings
            .iter()
            .filter(|binding| {
                binding
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
            })
            .flat_map(|binding| relation_ids(index, binding, completion_context))
            .collect::<Vec<_>>();
        if !alias_parents.is_empty() {
            return alias_parents
                .into_iter()
                .flat_map(|parent| index.children.get(&parent).into_iter().flatten().copied())
                .collect();
        }
    }
    let mut parents = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry.qualified_name.object.eq_ignore_ascii_case(qualifier)
                && (entry.kind == CatalogKind::Database
                    || entry.kind == CatalogKind::Schema
                    || entry.kind.is_relation())
        })
        .filter(|(_, entry)| catalog_entry_navigable(entry, dialect, completion_context))
        .map(|(_, entry)| entry.id.clone())
        .collect::<Vec<_>>();
    if parents.is_empty() && qualifiers.len() == 1 {
        return index
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.kind.is_relation()
                    && (entry
                        .qualified_name
                        .schema
                        .as_deref()
                        .is_some_and(|schema| schema.eq_ignore_ascii_case(qualifier))
                        || entry
                            .qualified_name
                            .database
                            .as_deref()
                            .is_some_and(|database| database.eq_ignore_ascii_case(qualifier)))
            })
            .filter(|(_, entry)| catalog_entry_navigable(entry, dialect, completion_context))
            .map(|(position, _)| position)
            .collect();
    }
    if let Some(current_database) = completion_context.database
        && dialect != SqlDialect::Postgres
    {
        let preferred = parents
            .iter()
            .filter(|parent| {
                index.entries.iter().any(|entry| {
                    entry.id == **parent
                        && entry
                            .qualified_name
                            .database
                            .as_deref()
                            .is_some_and(|database| database.eq_ignore_ascii_case(current_database))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            parents = preferred;
        }
    }
    for qualifier in &qualifiers[1..] {
        parents = parents
            .into_iter()
            .flat_map(|parent| index.children.get(&parent).into_iter().flatten())
            .filter_map(|position| {
                let entry = &index.entries[*position];
                entry
                    .qualified_name
                    .object
                    .eq_ignore_ascii_case(qualifier)
                    .then(|| entry.id.clone())
            })
            .collect();
    }
    let children = parents
        .into_iter()
        .flat_map(|parent| index.children.get(&parent).into_iter().flatten().copied())
        .collect::<Vec<_>>();
    if matches!(dialect, SqlDialect::MySql | SqlDialect::Sqlite) && qualifiers.len() == 1 {
        return fold_mirrored_schema_children(index, children);
    }
    children
}

fn catalog_entry_navigable(
    entry: &CatalogEntry,
    dialect: SqlDialect,
    completion_context: CompletionContext<'_>,
) -> bool {
    if dialect != SqlDialect::Postgres {
        return true;
    }
    let database = entry.qualified_name.database.as_deref();
    match completion_context.database {
        Some(current) => {
            if entry.kind == CatalogKind::Database {
                database.is_some_and(|value| value.eq_ignore_ascii_case(current))
            } else {
                database.is_none_or(|value| value.eq_ignore_ascii_case(current))
            }
        }
        None => true,
    }
}

fn fold_mirrored_schema_children(index: &CompletionIndex, children: Vec<usize>) -> Vec<usize> {
    let mut result = Vec::new();
    for position in children {
        let entry = &index.entries[position];
        let mirrored = entry.kind == CatalogKind::Schema
            && entry.id.native_path.len() == 2
            && entry
                .id
                .native_path
                .first()
                .zip(entry.id.native_path.get(1))
                .is_some_and(|(database, schema)| database.eq_ignore_ascii_case(schema));
        if mirrored {
            result.extend(index.children.get(&entry.id).into_iter().flatten().copied());
        } else {
            result.push(position);
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

fn ddl_candidate_indices(
    index: &CompletionIndex,
    qualifiers: &[String],
    prefix: &str,
) -> Vec<usize> {
    let mut candidates = candidate_indices(index, None, prefix);
    if qualifiers.is_empty() {
        return candidates;
    }
    candidates.retain(|position| {
        let entry = &index.entries[*position];
        let namespace = [
            entry.qualified_name.database.as_deref(),
            entry.qualified_name.schema.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        qualifiers.len() <= namespace.len()
            && qualifiers
                .iter()
                .zip(namespace)
                .all(|(qualifier, component)| qualifier.eq_ignore_ascii_case(component))
    });
    candidates
}

fn relation_ids(
    index: &CompletionIndex,
    binding: &RelationBinding,
    completion_context: CompletionContext<'_>,
) -> Vec<CatalogId> {
    let Some(object) = binding.name.last() else {
        return Vec::new();
    };
    let mut matches = index
        .entries
        .iter()
        .filter(|entry| {
            if !entry.kind.is_relation()
                || !entry.qualified_name.object.eq_ignore_ascii_case(object)
            {
                return false;
            }
            match binding.name.as_slice() {
                [database, schema, _] => {
                    entry
                        .qualified_name
                        .database
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(database))
                        && entry
                            .qualified_name
                            .schema
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case(schema))
                }
                [qualifier, _] => {
                    entry
                        .qualified_name
                        .schema
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(qualifier))
                        || entry
                            .qualified_name
                            .database
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case(qualifier))
                }
                [_] => true,
                _ => false,
            }
        })
        .collect::<Vec<_>>();
    if binding.name.len() == 1 {
        let preferred = matches
            .iter()
            .copied()
            .filter(|entry| {
                completion_context.database.is_none_or(|database| {
                    entry
                        .qualified_name
                        .database
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(database))
                }) && completion_context.schema.is_none_or(|schema| {
                    entry
                        .qualified_name
                        .schema
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(schema))
                })
            })
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            matches = preferred;
        }
    }
    matches.into_iter().map(|entry| entry.id.clone()).collect()
}

fn current_statement(text: &str, cursor: usize, dialect: SqlDialect) -> (&str, usize) {
    let range = scan_statements(text, dialect)
        .into_iter()
        .filter(|range| range.start <= cursor)
        .rfind(|range| {
            cursor <= range.end
                || (!text[..range.end].ends_with(';')
                    && text
                        .get(range.end..cursor)
                        .is_some_and(|gap| gap.trim().is_empty()))
        })
        .map(|range| TextRange::new(range.start, range.end.max(cursor)))
        .unwrap_or_else(|| TextRange::new(0, text.len()));
    (
        text.get(range.start..range.end).unwrap_or(text),
        cursor.saturating_sub(range.start),
    )
}

fn relation_bindings(tokens: &[CompletionToken]) -> Vec<RelationBinding> {
    let mut bindings = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let Some(word) = token_word(Some(token)) else {
            index += 1;
            continue;
        };
        if token.quoted
            || !matches!(
                word.to_ascii_lowercase().as_str(),
                "from" | "join" | "update" | "into"
            )
        {
            index += 1;
            continue;
        }
        let comma_list = word.eq_ignore_ascii_case("from");
        index += 1;
        while let Some((binding, next)) = relation_binding_at(tokens, index) {
            bindings.push(binding);
            index = next;
            if comma_list
                && matches!(
                    tokens.get(index).map(|token| &token.kind),
                    Some(CompletionTokenKind::Comma)
                )
            {
                index += 1;
            } else {
                break;
            }
        }
    }
    bindings
}

fn relation_binding_at(
    tokens: &[CompletionToken],
    start: usize,
) -> Option<(RelationBinding, usize)> {
    let first = tokens.get(start)?;
    let depth = first.depth;
    let mut name = vec![token_word(Some(first))?.to_owned()];
    let mut index = start + 1;
    while matches!(
        tokens.get(index).map(|token| &token.kind),
        Some(CompletionTokenKind::Dot)
    ) && tokens.get(index).is_some_and(|token| token.depth == depth)
    {
        let component = tokens.get(index + 1)?;
        if component.depth != depth {
            break;
        }
        name.push(token_word(Some(component))?.to_owned());
        index += 2;
    }
    let mut alias = None;
    if tokens.get(index).is_some_and(|token| !token.quoted)
        && token_word(tokens.get(index)).is_some_and(|word| word.eq_ignore_ascii_case("as"))
    {
        alias = token_word(tokens.get(index + 1)).map(str::to_owned);
        if alias.is_some() {
            index += 2;
        }
    } else if let Some(candidate) = token_word(tokens.get(index))
        && tokens
            .get(index)
            .is_some_and(|token| token.quoted || !is_relation_boundary(candidate))
        && tokens.get(index).is_some_and(|token| token.depth == depth)
    {
        alias = Some(candidate.to_owned());
        index += 1;
    }
    Some((
        RelationBinding {
            name,
            alias,
            depth,
            scope_start: first.scope_start,
        },
        index,
    ))
}

fn active_scope_starts(tokens: &[CompletionToken], cursor: usize) -> Vec<Option<usize>> {
    let mut scopes = vec![None];
    for token in tokens.iter().filter(|token| token.start < cursor) {
        match token.kind {
            CompletionTokenKind::LeftParen => scopes.push(Some(token.start)),
            CompletionTokenKind::RightParen if scopes.len() > 1 => {
                scopes.pop();
            }
            _ => {}
        }
    }
    scopes
}

fn visible_relation_bindings(
    tokens: &[CompletionToken],
    active_scopes: &[Option<usize>],
) -> Vec<RelationBinding> {
    relation_bindings(tokens)
        .into_iter()
        .filter(|binding| active_scopes.contains(&binding.scope_start))
        .collect()
}

fn is_relation_boundary(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "where"
            | "set"
            | "join"
            | "left"
            | "right"
            | "full"
            | "inner"
            | "cross"
            | "on"
            | "group"
            | "order"
            | "limit"
            | "having"
            | "returning"
            | "union"
            | "intersect"
            | "except"
    )
}

fn token_word(token: Option<&CompletionToken>) -> Option<&str> {
    match &token?.kind {
        CompletionTokenKind::Word(word) => Some(word),
        _ => None,
    }
}

fn completion_tokens(text: &str, dialect: SqlDialect) -> Vec<CompletionToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut depth = 0;
    let mut scope_starts = Vec::new();
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if bytes[index] == b'-' && bytes.get(index + 1) == Some(&b'-') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index < bytes.len() {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'\''
            || dialect == SqlDialect::SqlServer
                && matches!(bytes[index], b'N' | b'n')
                && bytes.get(index + 1) == Some(&b'\'')
        {
            let start = index;
            if bytes[index] != b'\'' {
                index += 1;
            }
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                    } else {
                        index += 1;
                        break;
                    }
                } else {
                    index += 1;
                }
            }
            tokens.push(CompletionToken {
                kind: CompletionTokenKind::Literal,
                start,
                end: index,
                depth,
                scope_start: scope_starts.last().copied(),
                quoted: false,
            });
            continue;
        }
        let quote = match bytes[index] {
            b'"' if !matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse) => Some(b'"'),
            b'[' if dialect == SqlDialect::SqlServer => Some(b']'),
            b'`' if matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse) => Some(b'`'),
            _ => None,
        };
        if let Some(quote) = quote {
            let start = index;
            index += 1;
            let content_start = index;
            let mut value = String::new();
            while index < bytes.len() {
                if bytes[index] == quote {
                    value.push_str(&text[content_start..index]);
                    if bytes.get(index + 1) == Some(&quote) {
                        value.push(quote as char);
                        index += 2;
                        let escaped_start = index;
                        while index < bytes.len() && bytes[index] != quote {
                            index += 1;
                        }
                        value.push_str(&text[escaped_start..index]);
                        continue;
                    }
                    index += 1;
                    break;
                }
                index += 1;
            }
            if value.is_empty() && content_start < index.saturating_sub(1) {
                value.push_str(&text[content_start..index.saturating_sub(1)]);
            }
            tokens.push(CompletionToken {
                kind: CompletionTokenKind::Word(value),
                start,
                end: index,
                depth,
                scope_start: scope_starts.last().copied(),
                quoted: true,
            });
            continue;
        }
        let punctuation = match bytes[index] {
            b'.' => Some(CompletionTokenKind::Dot),
            b',' => Some(CompletionTokenKind::Comma),
            b'(' => Some(CompletionTokenKind::LeftParen),
            b')' => Some(CompletionTokenKind::RightParen),
            b'[' if dialect != SqlDialect::SqlServer => Some(CompletionTokenKind::LeftBracket),
            b']' if dialect != SqlDialect::SqlServer => Some(CompletionTokenKind::RightBracket),
            b'*' => Some(CompletionTokenKind::Star),
            b'=' => Some(CompletionTokenKind::Equals),
            b'+' | b'-' | b'/' | b'%' | b'<' | b'>' | b'!' | b'|' | b'&' | b'^' | b':' => {
                Some(CompletionTokenKind::Operator)
            }
            _ => None,
        };
        if let Some(kind) = punctuation {
            let start = index;
            if kind == CompletionTokenKind::RightParen {
                depth = depth.saturating_sub(1);
                scope_starts.pop();
            }
            index += 1;
            tokens.push(CompletionToken {
                kind: kind.clone(),
                start,
                end: index,
                depth,
                scope_start: scope_starts.last().copied(),
                quoted: false,
            });
            if kind == CompletionTokenKind::LeftParen {
                depth += 1;
                scope_starts.push(start);
            }
            continue;
        }
        if is_identifier_byte(bytes[index], dialect) {
            let start = index;
            while index < bytes.len() && is_identifier_byte(bytes[index], dialect) {
                index += 1;
            }
            tokens.push(CompletionToken {
                kind: CompletionTokenKind::Word(text[start..index].to_owned()),
                start,
                end: index,
                depth,
                scope_start: scope_starts.last().copied(),
                quoted: false,
            });
            continue;
        }
        index += 1;
    }
    tokens
}

fn is_identifier_byte(byte: u8, dialect: SqlDialect) -> bool {
    byte.is_ascii_alphanumeric()
        || byte == b'_'
        || byte >= 0x80
        || matches!(byte, b'"' | b'`')
        || dialect == SqlDialect::SqlServer && matches!(byte, b'[' | b']' | b'@')
}

pub fn quote_identifier(value: &str, dialect: SqlDialect) -> String {
    if dialect == SqlDialect::SqlServer {
        return format!("[{}]", value.replace(']', "]]"));
    }
    let (quote, escaped) = if matches!(dialect, SqlDialect::MySql | SqlDialect::ClickHouse) {
        ('`', value.replace('`', "``"))
    } else {
        ('"', value.replace('"', "\"\""))
    };
    format!("{quote}{escaped}{quote}")
}

fn keywords(
    context: Context,
    dialect: SqlDialect,
    projection_complete: bool,
) -> &'static [&'static str] {
    match context {
        Context::Statement => match dialect {
            SqlDialect::MySql | SqlDialect::MariaDb => &[
                "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "TRUNCATE",
            ],
            _ => &[
                "SELECT", "WITH", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP",
                "TRUNCATE",
            ],
        },
        Context::Insert => &["INTO"],
        Context::UpdateSet => &["SET"],
        Context::AssignmentTarget => &[],
        Context::Expression(ExpressionContext::AssignmentValue) => {
            &["CASE", "NULL", "TRUE", "FALSE", "DEFAULT"]
        }
        Context::Delete(DeleteContext::From) => &["FROM"],
        Context::Delete(DeleteContext::AfterTarget) => &["WHERE"],
        Context::Delete(DeleteContext::Alias) => &[],
        Context::Ddl(DdlContext::CreateObjectKind) => ddl_object_keywords(dialect, true),
        Context::Ddl(DdlContext::AlterObjectKind) => &["TABLE", "VIEW", "INDEX", "SCHEMA"],
        Context::Ddl(DdlContext::DropObjectKind) => ddl_object_keywords(dialect, false),
        Context::Ddl(DdlContext::ExistingObject(_)) => &[],
        Context::Ddl(DdlContext::CreateIndexTarget) => &[],
        Context::Ddl(DdlContext::ColumnType) => &[],
        Context::Ddl(DdlContext::AlterColumnDefinition) => match dialect {
            SqlDialect::Postgres => &[
                "TYPE",
                "SET DEFAULT",
                "DROP DEFAULT",
                "SET NOT NULL",
                "DROP NOT NULL",
                "NULL",
                "NOT NULL",
                "DEFAULT",
            ],
            SqlDialect::SqlServer => &["NULL", "NOT NULL"],
            SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::Generic => &[
                "NULL",
                "NOT NULL",
                "DEFAULT",
                "PRIMARY KEY",
                "UNIQUE",
                "REFERENCES",
                "CHECK",
            ],
            SqlDialect::Sqlite
            | SqlDialect::DuckDb
            | SqlDialect::Oracle
            | SqlDialect::ClickHouse
            | SqlDialect::Snowflake => &[],
        },
        Context::Ddl(DdlContext::AlterColumnType(_)) => &[],
        Context::Ddl(DdlContext::NewColumnName) => &[],
        Context::Ddl(DdlContext::RenameColumnTo) => &["TO"],
        Context::Ddl(DdlContext::NewObjectName) => &[],
        Context::Ddl(DdlContext::TableConstraint) => &[
            "CONSTRAINT",
            "PRIMARY KEY",
            "UNIQUE",
            "FOREIGN KEY",
            "CHECK",
        ],
        Context::Ddl(DdlContext::AlterTableAction) => alter_table_action_keywords(dialect),
        Context::Ddl(
            DdlContext::ExistingColumn
            | DdlContext::ExistingConstraint
            | DdlContext::ExistingIndex
            | DdlContext::ReferenceRelation
            | DdlContext::ReferenceColumn,
        ) => &[],
        Context::Ddl(DdlContext::ColumnConstraint) => &[
            "NULL",
            "NOT NULL",
            "DEFAULT",
            "PRIMARY KEY",
            "UNIQUE",
            "REFERENCES",
            "CHECK",
        ],
        Context::Ddl(DdlContext::DefaultValue) => &["NULL", "TRUE", "FALSE"],
        Context::Expression(ExpressionContext::Projection) if projection_complete => {
            &["FROM", "CASE", "NULL", "TRUE", "FALSE"]
        }
        Context::Expression(ExpressionContext::Projection) => {
            &["DISTINCT", "CASE", "NULL", "TRUE", "FALSE"]
        }
        Context::Expression(ExpressionContext::Predicate) => &[
            "AND", "OR", "NOT", "EXISTS", "IN", "IS", "NULL", "LIKE", "BETWEEN", "CASE", "TRUE",
            "FALSE",
        ],
        Context::Expression(ExpressionContext::Grouping) => &["HAVING", "CASE", "NULL"],
        Context::Expression(ExpressionContext::Ordering) => match dialect {
            SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::SqlServer => &["ASC", "DESC"],
            _ => &["ASC", "DESC", "NULLS FIRST", "NULLS LAST"],
        },
        Context::Expression(ExpressionContext::Returning) => &["CASE", "NULL", "TRUE", "FALSE"],
        Context::Relation => &["LATERAL"],
        Context::Qualifier | Context::Routine => &[],
    }
}

fn ddl_object_keywords(dialect: SqlDialect, _create: bool) -> &'static [&'static str] {
    match dialect {
        SqlDialect::Postgres => &[
            "TABLE",
            "VIEW",
            "INDEX",
            "SCHEMA",
            "DATABASE",
            "MATERIALIZED VIEW",
            "SEQUENCE",
            "TYPE",
            "FUNCTION",
            "PROCEDURE",
            "TRIGGER",
        ],
        SqlDialect::MySql | SqlDialect::MariaDb | SqlDialect::SqlServer => &[
            "TABLE",
            "VIEW",
            "INDEX",
            "SCHEMA",
            "DATABASE",
            "FUNCTION",
            "PROCEDURE",
            "TRIGGER",
        ],
        SqlDialect::Sqlite => &["TABLE", "VIEW", "INDEX", "TRIGGER"],
        SqlDialect::DuckDb => &[
            "TABLE", "VIEW", "INDEX", "SCHEMA", "SEQUENCE", "TYPE", "MACRO", "FUNCTION",
        ],
        SqlDialect::Generic => &["TABLE", "VIEW", "INDEX", "SCHEMA", "DATABASE"],
        SqlDialect::Oracle => &[
            "TABLE",
            "VIEW",
            "INDEX",
            "SEQUENCE",
            "SYNONYM",
            "FUNCTION",
            "PROCEDURE",
            "PACKAGE",
            "TRIGGER",
            "TYPE",
        ],
        SqlDialect::Snowflake => &[
            "DATABASE",
            "SCHEMA",
            "TABLE",
            "VIEW",
            "FUNCTION",
            "PROCEDURE",
            "SEQUENCE",
            "WAREHOUSE",
        ],
        SqlDialect::ClickHouse => &["DATABASE", "DICTIONARY", "FUNCTION", "TABLE", "VIEW"],
    }
}

fn alter_table_action_continuation(
    tokens: &[CompletionToken],
    cursor: usize,
    prefix: &str,
) -> Option<&'static str> {
    if !"column".starts_with(&prefix.to_ascii_lowercase()) {
        return None;
    }
    let previous = tokens
        .iter()
        .rev()
        .find(|token| token.end <= cursor)
        .and_then(|token| token_word(Some(token)))?;
    matches!(
        previous.to_ascii_lowercase().as_str(),
        "add" | "modify" | "change"
    )
    .then_some("COLUMN")
}

fn alter_table_action_keywords(dialect: SqlDialect) -> &'static [&'static str] {
    match dialect {
        SqlDialect::MySql | SqlDialect::MariaDb => &[
            "ADD COLUMN",
            "MODIFY COLUMN",
            "CHANGE COLUMN",
            "DROP COLUMN",
            "ADD INDEX",
            "DROP INDEX",
            "ADD CONSTRAINT",
            "DROP FOREIGN KEY",
        ],
        SqlDialect::Postgres => &[
            "ADD COLUMN",
            "ALTER COLUMN",
            "DROP COLUMN",
            "RENAME COLUMN",
            "RENAME TO",
            "ADD CONSTRAINT",
            "DROP CONSTRAINT",
        ],
        SqlDialect::Sqlite | SqlDialect::DuckDb => {
            &["ADD COLUMN", "DROP COLUMN", "RENAME COLUMN", "RENAME TO"]
        }
        SqlDialect::SqlServer => &[
            "ADD",
            "ALTER COLUMN",
            "DROP COLUMN",
            "ADD CONSTRAINT",
            "DROP CONSTRAINT",
        ],
        SqlDialect::Generic | SqlDialect::Oracle => &[
            "ADD COLUMN",
            "ALTER COLUMN",
            "DROP COLUMN",
            "ADD CONSTRAINT",
            "DROP CONSTRAINT",
        ],
        SqlDialect::Snowflake => &[
            "ADD COLUMN",
            "DROP COLUMN",
            "RENAME COLUMN",
            "RENAME TO",
            "ALTER COLUMN",
        ],
        SqlDialect::ClickHouse => &[],
    }
}

fn data_types_for_context(context: Context, dialect: SqlDialect) -> &'static [&'static str] {
    if !matches!(
        context,
        Context::Ddl(
            DdlContext::ColumnType
                | DdlContext::AlterColumnDefinition
                | DdlContext::AlterColumnType(_),
        )
    ) {
        return &[];
    }
    match dialect {
        SqlDialect::Postgres => &[
            "BIGINT",
            "BOOLEAN",
            "DATE",
            "INTEGER",
            "JSONB",
            "NUMERIC",
            "TEXT",
            "TIMESTAMP",
            "TIMESTAMPTZ",
            "UUID",
            "VARCHAR",
        ],
        SqlDialect::MySql | SqlDialect::MariaDb => &[
            "BIGINT",
            "BOOLEAN",
            "DATETIME",
            "DECIMAL",
            "INT",
            "JSON",
            "TEXT",
            "TIMESTAMP",
            "VARCHAR",
        ],
        SqlDialect::SqlServer => &[
            "BIGINT",
            "BIT",
            "DATETIME2",
            "DECIMAL",
            "INT",
            "NVARCHAR",
            "UNIQUEIDENTIFIER",
            "VARCHAR",
        ],
        SqlDialect::Sqlite => &["BLOB", "INTEGER", "NUMERIC", "REAL", "TEXT"],
        SqlDialect::DuckDb => &[
            "BIGINT",
            "BOOLEAN",
            "DATE",
            "DECIMAL",
            "DOUBLE",
            "INTEGER",
            "JSON",
            "TIMESTAMP",
            "UBIGINT",
            "UUID",
            "VARCHAR",
        ],
        SqlDialect::Generic => &["BIGINT", "BOOLEAN", "INTEGER", "NUMERIC", "TEXT", "VARCHAR"],
        SqlDialect::Oracle => &[
            "BINARY_FLOAT",
            "BINARY_DOUBLE",
            "CHAR",
            "CLOB",
            "DATE",
            "NUMBER",
            "RAW",
            "TIMESTAMP",
            "VARCHAR2",
        ],
        SqlDialect::Snowflake => &[
            "NUMBER",
            "DECIMAL",
            "FLOAT",
            "VARCHAR",
            "BOOLEAN",
            "DATE",
            "TIME",
            "TIMESTAMP_NTZ",
            "TIMESTAMP_LTZ",
            "TIMESTAMP_TZ",
            "BINARY",
            "VARIANT",
            "OBJECT",
            "ARRAY",
        ],
        SqlDialect::ClickHouse => &[
            "Array",
            "Date",
            "Date32",
            "DateTime",
            "DateTime64",
            "Decimal",
            "FixedString",
            "Float32",
            "Float64",
            "Int8",
            "Int16",
            "Int32",
            "Int64",
            "Int128",
            "Nullable",
            "String",
            "Tuple",
            "UInt8",
            "UInt16",
            "UInt32",
            "UInt64",
            "UInt128",
            "UUID",
        ],
    }
}

fn display_text(value: &str) -> String {
    crate::security::sanitize_terminal_text(value)
}
