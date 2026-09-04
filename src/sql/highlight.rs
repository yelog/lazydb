use sqlparser::{
    ast::{
        AssignmentTarget, Expr, FromTable, ObjectName, ObjectNamePart, Query, Select, SelectItem,
        Statement, TableAlias, TableFactor, TableObject, TableWithJoins, UpdateTableFromKind,
        Visit, Visitor,
    },
    dialect::{
        Dialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
    },
    parser::Parser,
    tokenizer::{Token, TokenWithSpan, Tokenizer, Whitespace},
};

use std::{collections::HashMap, ops::ControlFlow};

use super::{LineIndex, SqlDialect, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightKind {
    Keyword,
    Identifier,
    Relation,
    RelationAlias,
    Column,
    Function,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Parameter,
    Plain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSpan {
    pub range: TextRange,
    pub kind: HighlightKind,
}

pub fn highlight_sql(text: &str, dialect: SqlDialect) -> Vec<HighlightSpan> {
    let mut spans = lexical_highlights(text, dialect);
    apply_semantic_highlights(text, dialect, &mut spans);
    merge_sql_server_variables(text, spans, dialect == SqlDialect::SqlServer)
}

fn lexical_highlights(text: &str, dialect: SqlDialect) -> Vec<HighlightSpan> {
    let index = LineIndex::new(text);
    let parser_dialect = dialect_ref(dialect);
    let mut tokenizer = Tokenizer::new(parser_dialect, text);
    let mut tokens = Vec::new();
    let _ = tokenizer.tokenize_with_location_into_buf(&mut tokens);
    tokens
        .into_iter()
        .filter_map(|token| span_for(text, &index, token, dialect))
        .collect::<Vec<_>>()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SemanticHighlight {
    range: TextRange,
    kind: HighlightKind,
}

const MAX_SEMANTIC_RECOVERY_ATTEMPTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BindingKind {
    Relation,
    RelationAlias,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RelationBinding {
    value: String,
    quote_style: Option<char>,
    kind: BindingKind,
}

struct SemanticCollector<'a> {
    text: &'a str,
    index: &'a LineIndex,
    spans: Vec<SemanticHighlight>,
    scopes: Vec<Vec<RelationBinding>>,
    statement_scopes: Vec<bool>,
    query_select_scopes: Vec<bool>,
}

impl SemanticCollector<'_> {
    fn push_ident(&mut self, ident: &sqlparser::ast::Ident, kind: HighlightKind) {
        let range = self
            .index
            .range(self.text, ident.span.start, ident.span.end);
        if !range.is_empty() {
            self.spans.push(SemanticHighlight { range, kind });
        }
    }

    fn push_object_name(&mut self, name: &ObjectName, kind: HighlightKind) {
        for ident in name.0.iter().filter_map(ObjectNamePart::as_ident) {
            self.push_ident(ident, kind);
        }
    }

    fn push_table_alias(&mut self, alias: &TableAlias) {
        self.push_ident(&alias.name, HighlightKind::RelationAlias);
        for column in &alias.columns {
            self.push_ident(&column.name, HighlightKind::Column);
        }
        if let Some(at) = &alias.at {
            self.push_ident(at, HighlightKind::RelationAlias);
        }
    }

    fn push_relation_binding(&mut self, ident: &sqlparser::ast::Ident, kind: BindingKind) {
        self.scopes.last_mut().into_iter().for_each(|scope| {
            scope.push(RelationBinding {
                value: ident.value.clone(),
                quote_style: ident.quote_style,
                kind,
            });
        });
    }

    fn push_select_aliases(&mut self, select: &Select) {
        for item in &select.projection {
            match item {
                SelectItem::ExprWithAlias { alias, .. } => {
                    self.push_ident(alias, HighlightKind::Column);
                }
                SelectItem::ExprWithAliases { aliases, .. } => {
                    for alias in aliases {
                        self.push_ident(alias, HighlightKind::Column);
                    }
                }
                SelectItem::UnnamedExpr(_)
                | SelectItem::QualifiedWildcard(..)
                | SelectItem::Wildcard(..) => {}
            }
        }
    }

    fn push_factor_binding(&mut self, factor: &TableFactor) {
        let (alias, relation) = match factor {
            TableFactor::Table { name, alias, .. } => (alias.as_ref(), Some(name)),
            TableFactor::Derived { alias, .. }
            | TableFactor::TableFunction { alias, .. }
            | TableFactor::Function { alias, .. }
            | TableFactor::UNNEST { alias, .. }
            | TableFactor::JsonTable { alias, .. }
            | TableFactor::OpenJsonTable { alias, .. }
            | TableFactor::NestedJoin { alias, .. }
            | TableFactor::Pivot { alias, .. }
            | TableFactor::Unpivot { alias, .. }
            | TableFactor::MatchRecognize { alias, .. }
            | TableFactor::XmlTable { alias, .. } => (alias.as_ref(), None),
            _ => (None, None),
        };

        if let Some(alias) = alias {
            self.push_relation_binding(&alias.name, BindingKind::RelationAlias);
            return;
        }

        if let Some(relation) = relation
            && let Some(ident) = relation.0.iter().rev().find_map(ObjectNamePart::as_ident)
        {
            self.push_relation_binding(ident, BindingKind::Relation);
        }
    }

    fn push_table_bindings(&mut self, table: &TableWithJoins) {
        self.push_factor_binding(&table.relation);
        for join in &table.joins {
            self.push_factor_binding(&join.relation);
        }
    }

    fn push_column_reference(&mut self, name: &ObjectName) {
        let identifiers = name
            .0
            .iter()
            .filter_map(ObjectNamePart::as_ident)
            .cloned()
            .collect::<Vec<_>>();
        self.push_compound_identifier(&identifiers);
    }

    fn push_assignment_target(&mut self, target: &AssignmentTarget) {
        match target {
            AssignmentTarget::ColumnName(name) => self.push_column_reference(name),
            AssignmentTarget::Tuple(names) => {
                for name in names {
                    self.push_column_reference(name);
                }
            }
        }
    }

    fn push_dml_bindings(&mut self, statement: &Statement) -> bool {
        match statement {
            Statement::Update(update) => {
                self.scopes.push(Vec::new());
                self.push_table_bindings(&update.table);
                if let Some(from) = &update.from {
                    let tables = match from {
                        UpdateTableFromKind::BeforeSet(tables)
                        | UpdateTableFromKind::AfterSet(tables) => tables,
                    };
                    for table in tables {
                        self.push_table_bindings(table);
                    }
                }
                for assignment in &update.assignments {
                    self.push_assignment_target(&assignment.target);
                }
                true
            }
            Statement::Delete(delete) => {
                self.scopes.push(Vec::new());
                let tables = match &delete.from {
                    FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => {
                        tables
                    }
                };
                for table in tables {
                    self.push_table_bindings(table);
                }
                if let Some(tables) = &delete.using {
                    for table in tables {
                        self.push_table_bindings(table);
                    }
                }
                true
            }
            Statement::Insert(insert) => {
                self.scopes.push(Vec::new());
                match &insert.table {
                    TableObject::TableName(name) => {
                        self.push_object_name(name, HighlightKind::Relation);
                        if let Some(ident) = name.0.iter().rev().find_map(ObjectNamePart::as_ident)
                        {
                            self.push_relation_binding(ident, BindingKind::Relation);
                        }
                    }
                    TableObject::TableFunction(function) => {
                        self.push_function_name(&function.name);
                    }
                    TableObject::TableQuery(_) => {}
                }
                if let Some(alias) = &insert.table_alias {
                    self.push_ident(&alias.alias, HighlightKind::RelationAlias);
                    self.push_relation_binding(&alias.alias, BindingKind::RelationAlias);
                }
                for column in &insert.columns {
                    self.push_column_reference(column);
                }
                true
            }
            _ => false,
        }
    }

    fn binding_kind(&self, ident: &sqlparser::ast::Ident) -> Option<BindingKind> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|binding| identifier_matches(binding, ident))
            .map(|binding| binding.kind)
    }

    fn push_compound_identifier(&mut self, identifiers: &[sqlparser::ast::Ident]) {
        let Some((column, qualifiers)) = identifiers.split_last() else {
            return;
        };
        self.push_ident(column, HighlightKind::Column);
        for qualifier in qualifiers {
            let Some(kind) = self.binding_kind(qualifier) else {
                continue;
            };
            self.push_ident(
                qualifier,
                match kind {
                    BindingKind::Relation => HighlightKind::Relation,
                    BindingKind::RelationAlias => HighlightKind::RelationAlias,
                },
            );
        }
    }

    fn push_function_name(&mut self, name: &ObjectName) {
        let Some((last, prefix)) = name.0.split_last() else {
            return;
        };
        for part in prefix {
            if let ObjectNamePart::Identifier(ident) = part {
                self.push_ident(ident, HighlightKind::Relation);
            }
        }
        if let ObjectNamePart::Identifier(ident) = last {
            self.push_ident(ident, HighlightKind::Function);
        }
    }
}

fn identifier_matches(binding: &RelationBinding, ident: &sqlparser::ast::Ident) -> bool {
    if binding.quote_style.is_some() || ident.quote_style.is_some() {
        binding.value == ident.value
    } else {
        binding.value.eq_ignore_ascii_case(&ident.value)
    }
}

impl Visitor for SemanticCollector<'_> {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        self.scopes.push(Vec::new());
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.push_ident(&cte.alias.name, HighlightKind::Relation);
                self.push_relation_binding(&cte.alias.name, BindingKind::Relation);
                for column in &cte.alias.columns {
                    self.push_ident(&column.name, HighlightKind::Column);
                }
            }
        }
        self.query_select_scopes.push(false);
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
        if self.query_select_scopes.pop().unwrap_or(false) {
            self.scopes.pop();
        }
        self.scopes.pop();
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        if self.query_select_scopes.last().copied().unwrap_or(false) {
            self.scopes.pop();
        }
        self.scopes.push(Vec::new());
        for table in &select.from {
            self.push_table_bindings(table);
        }
        self.push_select_aliases(select);
        if let Some(open) = self.query_select_scopes.last_mut() {
            *open = true;
        }
        ControlFlow::Continue(())
    }

    fn post_visit_select(&mut self, _select: &Select) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        self.push_object_name(relation, HighlightKind::Relation);
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        match table_factor {
            TableFactor::Table { alias, .. }
            | TableFactor::Derived { alias, .. }
            | TableFactor::TableFunction { alias, .. }
            | TableFactor::Function { alias, .. }
            | TableFactor::UNNEST { alias, .. }
            | TableFactor::JsonTable { alias, .. }
            | TableFactor::OpenJsonTable { alias, .. }
            | TableFactor::NestedJoin { alias, .. }
            | TableFactor::Pivot { alias, .. }
            | TableFactor::Unpivot { alias, .. }
            | TableFactor::MatchRecognize { alias, .. }
            | TableFactor::XmlTable { alias, .. } => {
                if let Some(alias) = alias {
                    self.push_table_alias(alias);
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<Self::Break> {
        let has_scope = self.push_dml_bindings(statement);
        self.statement_scopes.push(has_scope);
        ControlFlow::Continue(())
    }

    fn post_visit_statement(&mut self, _statement: &Statement) -> ControlFlow<Self::Break> {
        if self.statement_scopes.pop().unwrap_or(false) {
            self.scopes.pop();
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Identifier(ident) => self.push_ident(ident, HighlightKind::Column),
            Expr::CompoundIdentifier(identifiers) => self.push_compound_identifier(identifiers),
            Expr::Function(function) => self.push_function_name(&function.name),
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn semantic_highlights_for(source: &str, dialect: SqlDialect) -> Option<Vec<SemanticHighlight>> {
    let statements = Parser::parse_sql(dialect_ref(dialect), source).ok()?;
    let index = LineIndex::new(source);
    let mut collector = SemanticCollector {
        text: source,
        index: &index,
        spans: Vec::new(),
        scopes: Vec::new(),
        statement_scopes: Vec::new(),
        query_select_scopes: Vec::new(),
    };
    let _ = statements.visit(&mut collector);
    Some(collector.spans)
}

fn semantic_recovery_prefixes(source: &str, dialect: SqlDialect) -> Vec<usize> {
    let index = LineIndex::new(source);
    let mut tokenizer = Tokenizer::new(dialect_ref(dialect), source);
    let mut tokens = Vec::new();
    let _ = tokenizer.tokenize_with_location_into_buf(&mut tokens);

    let mut ends = tokens
        .into_iter()
        .filter_map(|token| {
            let start = index.range(source, token.span.start, token.span.end).start;
            (start > 0 && start <= source.len() && source.is_char_boundary(start)).then_some(start)
        })
        .collect::<Vec<_>>();
    ends.sort_unstable();
    ends.dedup();
    ends.into_iter()
        .rev()
        .take(MAX_SEMANTIC_RECOVERY_ATTEMPTS)
        .collect()
}

fn recover_semantic_highlights(
    source: &str,
    dialect: SqlDialect,
) -> Option<Vec<SemanticHighlight>> {
    if let Some(highlights) = semantic_highlights_for(source, dialect) {
        return Some(highlights);
    }

    semantic_recovery_prefixes(source, dialect)
        .into_iter()
        .filter_map(|end| source.get(..end).map(str::trim_end))
        .filter(|prefix| !prefix.is_empty())
        .find_map(|prefix| semantic_highlights_for(prefix, dialect))
}

fn apply_semantic_highlights(text: &str, dialect: SqlDialect, spans: &mut [HighlightSpan]) {
    let mut semantic = Vec::new();
    for statement in super::scan_statements(text, dialect) {
        let Some(source) = statement.get(text) else {
            continue;
        };
        let Some(highlights) = recover_semantic_highlights(source, dialect) else {
            continue;
        };
        semantic.extend(highlights.into_iter().map(|highlight| SemanticHighlight {
            range: TextRange::new(
                statement.start + highlight.range.start,
                statement.start + highlight.range.end,
            ),
            kind: highlight.kind,
        }));
    }

    apply_semantic_spans(spans, &semantic);
}

fn apply_semantic_spans(spans: &mut [HighlightSpan], semantic: &[SemanticHighlight]) {
    let mut overrides = HashMap::new();
    for highlight in semantic {
        let replace = overrides
            .get(&highlight.range)
            .is_none_or(|kind| semantic_priority(highlight.kind) > semantic_priority(*kind));
        if replace {
            overrides.insert(highlight.range, highlight.kind);
        }
    }

    for span in spans {
        if let Some(kind) = overrides.get(&span.range).copied()
            && can_override(span.kind, kind)
        {
            span.kind = kind;
        }
    }
}

fn semantic_priority(kind: HighlightKind) -> u8 {
    match kind {
        HighlightKind::RelationAlias => 4,
        HighlightKind::Function => 3,
        HighlightKind::Relation => 2,
        HighlightKind::Column => 1,
        _ => 0,
    }
}

fn can_override(current: HighlightKind, semantic: HighlightKind) -> bool {
    match semantic {
        HighlightKind::Function => {
            matches!(current, HighlightKind::Identifier | HighlightKind::Keyword)
        }
        HighlightKind::Relation | HighlightKind::RelationAlias | HighlightKind::Column => {
            matches!(current, HighlightKind::Identifier | HighlightKind::Keyword)
        }
        _ => false,
    }
}

fn merge_sql_server_variables(
    text: &str,
    spans: Vec<HighlightSpan>,
    sql_server: bool,
) -> Vec<HighlightSpan> {
    if !sql_server {
        return spans;
    }
    let mut merged: Vec<HighlightSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(previous) = merged.last_mut()
            && previous.kind == HighlightKind::Parameter
            && span.range.start == previous.range.end
            && (span.kind == HighlightKind::Identifier || span.kind == HighlightKind::Parameter)
            && previous
                .range
                .get(text)
                .is_some_and(|value| value.starts_with('@'))
        {
            previous.range.end = span.range.end;
            continue;
        }
        merged.push(span);
    }
    merged
}

pub fn highlight_sql_ranges(
    text: &str,
    ranges: &[TextRange],
    dialect: SqlDialect,
) -> Vec<HighlightSpan> {
    ranges
        .iter()
        .filter_map(|range| text.get(range.start..range.end).map(|sql| (*range, sql)))
        .flat_map(|(range, sql)| {
            highlight_sql(sql, dialect)
                .into_iter()
                .map(move |span| HighlightSpan {
                    range: TextRange::new(
                        range.start + span.range.start,
                        range.start + span.range.end,
                    ),
                    kind: span.kind,
                })
        })
        .collect()
}

fn span_for(
    text: &str,
    index: &LineIndex,
    token: TokenWithSpan,
    dialect: SqlDialect,
) -> Option<HighlightSpan> {
    let range = index.range(text, token.span.start, token.span.end);
    if range.is_empty() {
        return None;
    }
    let kind = match token.token {
        Token::Word(word) if word.keyword != sqlparser::keywords::Keyword::NoKeyword => {
            HighlightKind::Keyword
        }
        Token::Word(_)
            if dialect == SqlDialect::SqlServer
                && range.get(text).is_some_and(|value| value.starts_with('@')) =>
        {
            HighlightKind::Parameter
        }
        Token::Word(_) => HighlightKind::Identifier,
        Token::SingleQuotedString(_)
        | Token::NationalStringLiteral(_)
        | Token::DoubleQuotedString(_)
        | Token::DollarQuotedString(_)
        | Token::EscapedStringLiteral(_) => HighlightKind::String,
        Token::Number(_, _) => HighlightKind::Number,
        Token::Whitespace(
            Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_),
        ) => HighlightKind::Comment,
        Token::Whitespace(_) => return None,
        Token::AtSign | Token::Colon | Token::Placeholder(_) => HighlightKind::Parameter,
        Token::Comma | Token::LParen | Token::RParen | Token::Period | Token::SemiColon => {
            HighlightKind::Punctuation
        }
        Token::EOF => return None,
        _ => HighlightKind::Operator,
    };
    Some(HighlightSpan { range, kind })
}

fn dialect_ref(dialect: SqlDialect) -> &'static dyn Dialect {
    match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::MySql => &MySqlDialect {},
        SqlDialect::SqlServer => &MsSqlDialect {},
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::Generic => &GenericDialect {},
    }
}
