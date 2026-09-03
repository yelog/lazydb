use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

use crate::db::catalog::{CatalogEntry, CatalogId, CatalogKind, CatalogMetadata};
use crate::profile::CatalogScope;

use super::identifier_match::{
    IdentifierMatch, compact_identifier, fold_identifier, identifier_match,
};
use super::scope::scan_statements;
use super::{SqlDialect, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionScheduleKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub connection: crate::model::workspace::ConnectionIdentity,
    pub catalog_generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompletionContext<'a> {
    pub database: Option<&'a str>,
    pub schema: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CompletionKind {
    Keyword,
    Database,
    Schema,
    Table,
    View,
    Column,
    Function,
    Procedure,
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
    InsertColumns,
    Relation,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionContext {
    Projection,
    Predicate,
    Grouping,
    Ordering,
    Returning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CompletionTokenKind {
    Word(String),
    Literal,
    Operator,
    Star,
    Dot,
    Comma,
    LeftParen,
    RightParen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionToken {
    kind: CompletionTokenKind,
    start: usize,
    end: usize,
    depth: usize,
    scope_start: Option<usize>,
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
    let cursor = cursor.min(text.len());
    let (replace, prefix, qualifiers) = identifier_at(text, cursor, dialect);
    let (statement, statement_cursor) = current_statement(text, replace.start, dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    let current_scope = active_scopes.last().copied().flatten();
    let insert_column_target = insert_column_target_at(&tokens, statement_cursor, current_scope);
    let context = if insert_column_target.is_some() {
        Context::InsertColumns
    } else {
        context_at(&tokens, statement_cursor, current_scope)
    };
    let insert_target_relations = insert_column_target
        .as_ref()
        .into_iter()
        .flat_map(|binding| relation_ids(index, binding, completion_context))
        .collect::<HashSet<_>>();
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
    let candidate_indexes = if context == Context::InsertColumns {
        if qualifiers.is_empty() {
            relation_child_candidate_indices(index, &insert_target_relations)
        } else {
            Vec::new()
        }
    } else {
        qualified_candidate_indices(
            index,
            &qualifiers,
            &folded_prefix,
            &bindings,
            completion_context,
        )
    };
    for node_index in candidate_indexes {
        let entry = &index.entries[node_index];
        let Some(kind) = completion_kind(entry.kind) else {
            continue;
        };
        if !catalog_kind_allowed(context, kind) {
            continue;
        }
        if kind == CompletionKind::Column
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
        let name = &entry.qualified_name.object;
        let Some(name_match) = identifier_match(name, &prefix) else {
            continue;
        };
        let context_score = match (context, kind) {
            (Context::Relation, CompletionKind::Table | CompletionKind::View)
            | (Context::Qualifier, CompletionKind::Column)
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
                relation_insert_text(entry, completion_context, dialect, &qualifiers)
            } else {
                quote_identifier(name, dialect)
            },
            kind,
            detail: if matches!(kind, CompletionKind::Table | CompletionKind::View) {
                relation_detail(entry)
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
        for keyword in keywords(context, dialect, projection_complete) {
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
                            (Context::Statement | Context::Insert, _, _) => 4,
                            (Context::Expression(_), _, _) => 2,
                            (Context::Relation | Context::Routine, _, _) => 1,
                            (Context::Qualifier | Context::InsertColumns, _, _) => 0,
                        },
                        name_match: 2,
                        schema: 0,
                    },
                });
            }
        }
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.label.cmp(&right.label))
    });
    candidates.truncate(10);
    candidates
}

pub fn relation_ids_for_completion(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
    index: &CompletionIndex,
    completion_context: CompletionContext<'_>,
) -> Vec<CatalogId> {
    let cursor = cursor.min(text.len());
    let (statement, statement_cursor) = current_statement(text, cursor, dialect);
    let tokens = completion_tokens(statement, dialect);
    let active_scopes = active_scope_starts(&tokens, statement_cursor);
    visible_relation_bindings(&tokens, &active_scopes)
        .into_iter()
        .flat_map(|binding| relation_ids(index, &binding, completion_context))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn completion_detail(entry: &CatalogEntry) -> Option<String> {
    match &entry.metadata {
        CatalogMetadata::Column(column) => Some(column.native_type.clone()),
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
        SqlDialect::MySql => {
            if database.is_some_and(|value| context.database == Some(value)) {
                vec![object]
            } else {
                vec![database.unwrap_or_default(), object]
            }
        }
        SqlDialect::Sqlite => {
            if schema.is_some_and(|value| context.schema == Some(value)) {
                vec![object]
            } else {
                vec![schema.or(database).unwrap_or_default(), object]
            }
        }
        SqlDialect::Postgres | SqlDialect::SqlServer | SqlDialect::Generic => {
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
    };
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| quote_relation_component(part, dialect))
        .collect::<Vec<_>>()
        .join(".")
}

fn quote_relation_component(value: &str, dialect: SqlDialect) -> String {
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

pub fn should_offer_completion(text: &str, cursor: usize) -> bool {
    let cursor = cursor.min(text.len());
    let Some(previous) = cursor
        .checked_sub(1)
        .and_then(|index| text.as_bytes().get(index))
    else {
        return false;
    };
    if *previous == b'.'
        || previous.is_ascii_alphanumeric()
        || *previous == b'_'
        || *previous >= 0x80
    {
        return true;
    }
    if previous.is_ascii_whitespace() {
        let before = text[..cursor].trim_end().to_ascii_lowercase();
        return [
            "from", "join", "update", "insert", "into", "select", "where",
        ]
        .iter()
        .any(|keyword| before.ends_with(keyword));
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

fn relation_child_candidate_indices(
    index: &CompletionIndex,
    relations: &HashSet<CatalogId>,
) -> Vec<usize> {
    relations
        .iter()
        .flat_map(|relation| index.children.get(relation).into_iter().flatten().copied())
        .collect()
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
        CatalogKind::Function => CompletionKind::Function,
        CatalogKind::Procedure => CompletionKind::Procedure,
        _ => return None,
    })
}

fn catalog_kind_allowed(context: Context, kind: CompletionKind) -> bool {
    match context {
        Context::Statement | Context::Insert => false,
        Context::InsertColumns => kind == CompletionKind::Column,
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
    }
}

fn context_at(tokens: &[CompletionToken], cursor: usize, current_scope: Option<usize>) -> Context {
    let tokens = tokens
        .iter()
        .filter(|token| token.end <= cursor && token.scope_start == current_scope)
        .collect::<Vec<_>>();
    let mut context = Context::Statement;
    for (index, token) in tokens.iter().enumerate() {
        let CompletionTokenKind::Word(word) = &token.kind else {
            continue;
        };
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
            _ => context,
        };
    }
    if matches!(
        tokens.last().map(|token| &token.kind),
        Some(CompletionTokenKind::Dot)
    ) && context != Context::Relation
    {
        Context::Qualifier
    } else {
        context
    }
}

fn insert_column_target_at(
    tokens: &[CompletionToken],
    cursor: usize,
    current_scope: Option<usize>,
) -> Option<RelationBinding> {
    let opening_start = current_scope?;
    let opening_index = tokens.iter().position(|token| {
        token.start == opening_start
            && token.start < cursor
            && token.kind == CompletionTokenKind::LeftParen
    })?;
    let parent_scope = tokens[opening_index].scope_start;
    let insert_index = tokens[..opening_index].iter().rposition(|token| {
        token.scope_start == parent_scope
            && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("insert"))
    })?;
    let into_index = tokens[insert_index + 1..opening_index]
        .iter()
        .position(|token| {
            token.scope_start == parent_scope
                && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("into"))
        })?
        + insert_index
        + 1;
    let (binding, next) = relation_binding_at(tokens, into_index + 1)?;

    (binding.scope_start == parent_scope && next == opening_index).then_some(binding)
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
        | CompletionTokenKind::RightParen => true,
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
        | CompletionTokenKind::Dot
        | CompletionTokenKind::Comma
        | CompletionTokenKind::LeftParen => false,
    }
}

fn identifier_at(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
) -> (TextRange, String, Vec<String>) {
    let mut start = cursor;
    while start > 0 && is_identifier_byte(text.as_bytes()[start - 1], dialect) {
        start -= 1;
    }
    let prefix = text[start..cursor].to_owned();
    let qualifiers = if start > 0 && text.as_bytes()[start - 1] == b'.' {
        let mut q = start - 1;
        while q > 0
            && (is_identifier_byte(text.as_bytes()[q - 1], dialect)
                || text.as_bytes()[q - 1] == b'.')
        {
            q -= 1;
        }
        text[q..start - 1]
            .split('.')
            .map(|value| value.trim_matches(['"', '`', '[', ']']).to_owned())
            .filter(|value| !value.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    (
        TextRange::new(start, cursor),
        prefix.trim_matches(['"', '`', '[', ']']).to_owned(),
        qualifiers,
    )
}

fn qualified_candidate_indices(
    index: &CompletionIndex,
    qualifiers: &[String],
    prefix: &str,
    bindings: &[RelationBinding],
    completion_context: CompletionContext<'_>,
) -> Vec<usize> {
    if qualifiers.is_empty() {
        return candidate_indices(index, None, prefix);
    }
    let qualifier = &qualifiers[0];
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
    let mut parents = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .qualified_name
                .object
                .eq_ignore_ascii_case(&qualifiers[0])
                && (entry.kind == CatalogKind::Database
                    || entry.kind == CatalogKind::Schema
                    || entry.kind.is_relation())
        })
        .map(|(_, entry)| entry.id.clone())
        .collect::<Vec<_>>();
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
    parents
        .into_iter()
        .flat_map(|parent| index.children.get(&parent).into_iter().flatten().copied())
        .collect()
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
        .find(|range| range.start <= cursor && cursor <= range.end)
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
        if !matches!(
            word.to_ascii_lowercase().as_str(),
            "from" | "join" | "update" | "into"
        ) {
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
    if token_word(tokens.get(index)).is_some_and(|word| word.eq_ignore_ascii_case("as")) {
        alias = token_word(tokens.get(index + 1)).map(str::to_owned);
        if alias.is_some() {
            index += 2;
        }
    } else if let Some(candidate) = token_word(tokens.get(index))
        && !is_relation_boundary(candidate)
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
            | "values"
            | "select"
            | "default"
            | "overriding"
            | "set"
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
            });
            continue;
        }
        let quote = match bytes[index] {
            b'"' if dialect != SqlDialect::MySql => Some(b'"'),
            b'[' if dialect == SqlDialect::SqlServer => Some(b']'),
            b'`' if dialect == SqlDialect::MySql => Some(b'`'),
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
            });
            continue;
        }
        let punctuation = match bytes[index] {
            b'.' => Some(CompletionTokenKind::Dot),
            b',' => Some(CompletionTokenKind::Comma),
            b'(' => Some(CompletionTokenKind::LeftParen),
            b')' => Some(CompletionTokenKind::RightParen),
            b'*' => Some(CompletionTokenKind::Star),
            b'+' | b'-' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'|' | b'&' | b'^' | b':' => {
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
    let (quote, escaped) = if dialect == SqlDialect::MySql {
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
            SqlDialect::MySql => &["SELECT", "INSERT", "UPDATE", "DELETE"],
            _ => &["SELECT", "WITH", "INSERT", "UPDATE", "DELETE"],
        },
        Context::Insert => &["INTO"],
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
            SqlDialect::MySql => &["ASC", "DESC"],
            _ => &["ASC", "DESC", "NULLS FIRST", "NULLS LAST"],
        },
        Context::Expression(ExpressionContext::Returning) => &["CASE", "NULL", "TRUE", "FALSE"],
        Context::Relation => &["LATERAL"],
        Context::Qualifier | Context::Routine => &[],
        Context::InsertColumns => &[],
    }
}

fn display_text(value: &str) -> String {
    crate::security::sanitize_terminal_text(value)
}
