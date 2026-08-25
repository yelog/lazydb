use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use crate::db::catalog::{CatalogId, CatalogKind, CatalogNode};

use super::{SqlDialect, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionScheduleKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub connection: crate::model::workspace::ConnectionIdentity,
    pub catalog_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CompletionKind {
    Keyword,
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
    nodes: Vec<CatalogNode>,
}

impl CompletionIndex {
    pub fn new(nodes: &[CatalogNode]) -> Self {
        let mut index = Self {
            nodes: nodes.to_vec(),
            ..Self::default()
        };
        for (position, node) in nodes.iter().enumerate() {
            index
                .by_name
                .entry(fold(&node.name))
                .or_default()
                .push(position);
            if let Some(parent) = &node.parent_id {
                index
                    .children
                    .entry(parent.clone())
                    .or_default()
                    .push(position);
            }
        }
        index
    }

    pub fn nodes(&self) -> &[CatalogNode] {
        &self.nodes
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
    default_schema: Option<&str>,
) -> Vec<CompletionCandidate> {
    let cursor = cursor.min(text.len());
    let (replace, prefix, qualifier) = identifier_at(text, cursor, dialect);
    let context = context_at(text, replace.start, dialect);
    let parent = qualifier.as_deref().and_then(|name| {
        index
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.kind,
                    CatalogKind::Schema | CatalogKind::Table | CatalogKind::View
                ) && node.name.eq_ignore_ascii_case(name)
            })
            .map(|node| node.id.clone())
    });
    let mut candidates = Vec::new();
    let folded_prefix = fold(&prefix);
    for node_index in candidate_indices(index, parent.as_ref(), &folded_prefix) {
        let node = &index.nodes[node_index];
        let Some(kind) = completion_kind(node.kind) else {
            continue;
        };
        if dialect == SqlDialect::Sqlite
            && matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        {
            continue;
        }
        if !node.name.to_lowercase().starts_with(&folded_prefix) {
            continue;
        }
        if context == Context::Relation
            && !matches!(kind, CompletionKind::Table | CompletionKind::View)
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
        let schema_score = u8::from(default_schema.is_some_and(|schema| {
            node.id
                .native_path
                .iter()
                .any(|part| part.eq_ignore_ascii_case(schema))
        }));
        candidates.push(CompletionCandidate {
            label: display_text(&node.name),
            insert_text: quote_identifier(&node.name, dialect),
            kind,
            detail: node.detail.as_deref().map(display_text),
            replace,
            score: CompletionScore {
                context: context_score,
                prefix: u8::from(node.name.starts_with(&prefix)),
                schema: schema_score,
            },
        });
    }
    if qualifier.is_none() {
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
        CatalogKind::Schema => CompletionKind::Schema,
        CatalogKind::Table => CompletionKind::Table,
        CatalogKind::View => CompletionKind::View,
        CatalogKind::Column => CompletionKind::Column,
        CatalogKind::Function => CompletionKind::Function,
        CatalogKind::Procedure => CompletionKind::Procedure,
        _ => return None,
    })
}

fn context_at(text: &str, start: usize, dialect: SqlDialect) -> Context {
    let before = text[..start].to_ascii_lowercase();
    if before.ends_with('.') {
        return Context::Qualifier;
    }
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
) -> (TextRange, String, Option<String>) {
    let mut start = cursor;
    while start > 0 && is_identifier_byte(text.as_bytes()[start - 1], dialect) {
        start -= 1;
    }
    let prefix = text[start..cursor].to_owned();
    let qualifier = (start > 0 && text.as_bytes()[start - 1] == b'.')
        .then(|| {
            let mut q = start - 1;
            while q > 0 && is_identifier_byte(text.as_bytes()[q - 1], dialect) {
                q -= 1;
            }
            text[q..start - 1].trim_matches(['"', '`']).to_owned()
        })
        .filter(|value| !value.is_empty());
    (
        TextRange::new(start, cursor),
        prefix.trim_matches(['"', '`']).to_owned(),
        qualifier,
    )
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
