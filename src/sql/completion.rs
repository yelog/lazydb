use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use crate::db::catalog::{CatalogEntry, CatalogId, CatalogKind, CatalogMetadata};
use crate::profile::CatalogScope;

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
    pub prefix: u8,
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

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    fn rebuild(&mut self) {
        self.by_name.clear();
        self.children.clear();
        for (position, entry) in self.entries.iter().enumerate() {
            self.by_name
                .entry(fold(&entry.qualified_name.object))
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
    Relation,
    Qualifier,
    Routine,
    General,
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
    let context = context_at(text, replace.start, dialect);
    let statement = current_statement_text(text, replace.start, dialect);
    let bindings = relation_bindings(statement, dialect);
    let mut candidates = Vec::new();
    let folded_prefix = fold(&prefix);
    let candidate_indexes =
        qualified_candidate_indices(index, &qualifiers, &folded_prefix, &bindings);
    for node_index in candidate_indexes {
        let entry = &index.entries[node_index];
        let Some(kind) = completion_kind(entry.kind) else {
            continue;
        };
        if kind == CompletionKind::Column
            && qualifiers.is_empty()
            && !bindings.is_empty()
            && !entry_belongs_to_binding(entry, index, &bindings)
        {
            continue;
        }
        if dialect == SqlDialect::Sqlite
            && matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        {
            continue;
        }
        let name = &entry.qualified_name.object;
        if !name.to_lowercase().starts_with(&folded_prefix) {
            continue;
        }
        if context == Context::Relation
            && !matches!(
                kind,
                CompletionKind::Database
                    | CompletionKind::Schema
                    | CompletionKind::Table
                    | CompletionKind::View
            )
        {
            continue;
        }
        if context == Context::Qualifier
            && !matches!(
                kind,
                CompletionKind::Column | CompletionKind::Table | CompletionKind::View
            )
        {
            continue;
        }
        if context == Context::Routine
            && !matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        {
            continue;
        }
        let context_score = match (context, kind) {
            (Context::Relation, CompletionKind::Table | CompletionKind::View)
            | (Context::Qualifier, CompletionKind::Column)
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
            label: if matches!(kind, CompletionKind::Table | CompletionKind::View) {
                display_text(name)
            } else {
                display_text(name)
            },
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
                prefix: u8::from(name.starts_with(&prefix)),
                schema: schema_score,
            },
        });
    }
    if qualifiers.is_empty() {
        for keyword in keywords(dialect) {
            if keyword.to_lowercase().starts_with(&folded_prefix) {
                candidates.push(CompletionCandidate {
                    label: (*keyword).to_owned(),
                    insert_text: (*keyword).to_owned(),
                    kind: CompletionKind::Keyword,
                    detail: None,
                    replace,
                    score: CompletionScore {
                        context: match context {
                            Context::General => 4,
                            Context::Relation | Context::Routine => 1,
                            Context::Qualifier => 0,
                        },
                        prefix: 1,
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
) -> Vec<CatalogId> {
    let cursor = cursor.min(text.len());
    let statement = current_statement_text(text, cursor, dialect);
    relation_bindings(statement, dialect)
        .into_iter()
        .flat_map(|(relation, _)| relation_parents(index, &relation))
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
        _ => {
            if database.is_some_and(|value| context.database == Some(value)) {
                if schema.is_some_and(|value| context.schema == Some(value)) {
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
        return ["from", "join", "update", "into", "select", "where"]
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
    index
        .by_name
        .range(prefix.to_owned()..)
        .flat_map(|(_, values)| values.iter().copied())
        .collect()
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

fn context_at(text: &str, start: usize, dialect: SqlDialect) -> Context {
    let before = text[..start].to_ascii_lowercase();
    let word = before.split_whitespace().last().unwrap_or_default();
    let relation = [" from ", " join ", " update ", " into "]
        .iter()
        .filter_map(|keyword| before.rfind(keyword))
        .max();
    let clause = [" where ", " select ", " returning "]
        .iter()
        .filter_map(|keyword| before.rfind(keyword))
        .max();
    if matches!(word, "from" | "join" | "update" | "into") || relation > clause {
        return Context::Relation;
    }
    if before.ends_with('.') {
        return Context::Qualifier;
    }
    if before.ends_with("select ") || before.ends_with("select") {
        return Context::General;
    }
    let _ = dialect;
    if before.ends_with("call ") || before.ends_with("execute ") {
        Context::Routine
    } else {
        Context::General
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
            .map(|value| value.trim_matches(['"', '`']).to_owned())
            .filter(|value| !value.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    (
        TextRange::new(start, cursor),
        prefix.trim_matches(['"', '`']).to_owned(),
        qualifiers,
    )
}

fn qualified_candidate_indices(
    index: &CompletionIndex,
    qualifiers: &[String],
    prefix: &str,
    bindings: &[(String, Option<String>)],
) -> Vec<usize> {
    if qualifiers.is_empty() {
        return candidate_indices(index, None, prefix);
    }
    let qualifier = &qualifiers[0];
    let alias_parents = bindings
        .iter()
        .filter(|(_, alias)| {
            alias
                .as_deref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
        })
        .flat_map(|(relation, _)| relation_parents(index, relation))
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

fn relation_parents(index: &CompletionIndex, relation: &str) -> Vec<CatalogId> {
    index
        .entries
        .iter()
        .filter(|entry| {
            entry.qualified_name.object.eq_ignore_ascii_case(relation) && entry.kind.is_relation()
        })
        .map(|entry| entry.id.clone())
        .collect()
}

fn entry_belongs_to_binding(
    entry: &CatalogEntry,
    index: &CompletionIndex,
    bindings: &[(String, Option<String>)],
) -> bool {
    let Some(parent) = entry.parent_id.as_ref() else {
        return false;
    };
    index.entries.iter().any(|candidate| {
        candidate.id == *parent
            && candidate.kind.is_relation()
            && bindings.iter().any(|(relation, alias)| {
                candidate
                    .qualified_name
                    .object
                    .eq_ignore_ascii_case(relation)
                    || alias.as_deref().is_some_and(|alias| {
                        candidate.qualified_name.object.eq_ignore_ascii_case(alias)
                    })
            })
    })
}

fn current_statement_text(text: &str, cursor: usize, dialect: SqlDialect) -> &str {
    scan_statements(text, dialect)
        .into_iter()
        .find(|range| range.start <= cursor && cursor <= range.end)
        .and_then(|range| text.get(range.start..range.end))
        .unwrap_or(text)
}

fn relation_bindings(text: &str, dialect: SqlDialect) -> Vec<(String, Option<String>)> {
    let words = text
        .split(|character: char| !is_identifier_byte(character as u8, dialect) && character != '.')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut bindings = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if !matches!(
            word.to_ascii_lowercase().as_str(),
            "from" | "join" | "update" | "into"
        ) {
            continue;
        }
        let Some(relation) = words.get(index + 1) else {
            continue;
        };
        let relation = relation
            .trim_matches(['"', '`'])
            .split('.')
            .next_back()
            .unwrap_or(relation)
            .to_owned();
        let alias = words
            .get(index + 2)
            .filter(|candidate| {
                !matches!(
                    candidate.to_ascii_lowercase().as_str(),
                    "where" | "join" | "on" | "group" | "order" | "limit" | "having" | "returning"
                )
            })
            .map(|alias| alias.trim_matches(['"', '`']).to_owned());
        bindings.push((relation, alias));
    }
    bindings
}

fn is_identifier_byte(byte: u8, dialect: SqlDialect) -> bool {
    let _ = dialect;
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80 || matches!(byte, b'"' | b'`')
}

pub fn quote_identifier(value: &str, dialect: SqlDialect) -> String {
    let (quote, escaped) = if dialect == SqlDialect::MySql {
        ('`', value.replace('`', "``"))
    } else {
        ('"', value.replace('"', "\"\""))
    };
    format!("{quote}{escaped}{quote}")
}

fn keywords(dialect: SqlDialect) -> &'static [&'static str] {
    match dialect {
        SqlDialect::MySql => &[
            "SELECT", "FROM", "WHERE", "JOIN", "CALL", "INSERT", "UPDATE", "DELETE",
        ],
        _ => &[
            "SELECT",
            "FROM",
            "WHERE",
            "JOIN",
            "RETURNING",
            "WITH",
            "INSERT",
            "UPDATE",
            "DELETE",
        ],
    }
}

fn fold(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

fn display_text(value: &str) -> String {
    crate::security::sanitize_terminal_text(value)
}
