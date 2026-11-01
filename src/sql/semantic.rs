use std::collections::HashMap;
use std::ops::ControlFlow;

use crate::db::catalog::{CatalogEntry, CatalogId, CatalogKind};

use super::{SqlDialect, TextRange, analysis::LineIndex, dialect::parser_dialect};

/// Whether the catalog contains enough information to make a negative claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogCoverage {
    NotLoaded,
    Loading,
    Complete,
    Failed,
    OutOfScope,
}

impl CatalogCoverage {
    pub const fn can_prove_missing(self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CatalogNamespace {
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl CatalogNamespace {
    pub fn new(database: Option<impl Into<String>>, schema: Option<impl Into<String>>) -> Self {
        Self {
            database: database.map(Into::into),
            schema: schema.map(Into::into),
        }
    }
}

/// A read-only view of the catalog used by semantic analysis.
///
/// Entries and coverage deliberately live together. An empty entry list is
/// not evidence of a missing object until the corresponding namespace has
/// `Complete` coverage. For Snowflake, `(None, None)` covers the database
/// list, `(Some(database), None)` covers that database's schema list, and
/// `(Some(database), Some(schema))` covers relations within that schema.
#[derive(Clone, Debug, Default)]
pub struct CatalogSnapshot {
    entries: Vec<CatalogEntry>,
    coverage: HashMap<CatalogNamespace, CatalogCoverage>,
    column_coverage: HashMap<CatalogId, CatalogCoverage>,
}

impl CatalogSnapshot {
    pub fn new(
        entries: impl IntoIterator<Item = CatalogEntry>,
        coverage: impl IntoIterator<Item = (CatalogNamespace, CatalogCoverage)>,
    ) -> Self {
        let mut namespace_coverage = HashMap::new();
        for (namespace, status) in coverage {
            namespace_coverage
                .entry(namespace)
                .and_modify(|current| *current = merge_coverage(*current, status))
                .or_insert(status);
        }
        Self {
            entries: entries.into_iter().collect(),
            coverage: namespace_coverage,
            column_coverage: HashMap::new(),
        }
    }

    pub fn with_column_coverage(
        mut self,
        coverage: impl IntoIterator<Item = (CatalogId, CatalogCoverage)>,
    ) -> Self {
        for (relation, status) in coverage {
            self.column_coverage
                .entry(relation)
                .and_modify(|current| *current = merge_coverage(*current, status))
                .or_insert(status);
        }
        self
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn coverage(&self, namespace: &CatalogNamespace) -> CatalogCoverage {
        self.coverage
            .get(namespace)
            .copied()
            .unwrap_or(CatalogCoverage::NotLoaded)
    }

    pub fn relations(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(|entry| entry.kind.is_relation())
    }

    pub fn databases(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Database)
    }

    pub fn schemas(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Schema)
    }

    fn has_database(&self, name: &str, dialect: SqlDialect) -> bool {
        self.databases()
            .any(|entry| same_identifier(&entry.qualified_name.object, name, dialect))
    }

    fn has_schema(&self, database: Option<&str>, schema: &str, dialect: SqlDialect) -> bool {
        self.schemas().any(|entry| {
            same_identifier(&entry.qualified_name.object, schema, dialect)
                && same_optional_identifier(
                    entry.qualified_name.database.as_deref(),
                    database,
                    dialect,
                )
        })
    }

    pub fn relation_columns(&self, relation: &CatalogId) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(move |entry| {
            entry.kind == CatalogKind::Column
                && entry
                    .owning_relation_id()
                    .is_some_and(|owner| owner == relation)
        })
    }

    pub fn column_coverage(&self, relation: &CatalogId) -> CatalogCoverage {
        self.column_coverage
            .get(relation)
            .copied()
            .unwrap_or(CatalogCoverage::NotLoaded)
    }

    /// Resolves unquoted name components in the active dialect.
    /// Parsed SQL uses `resolve_relation_keys` to preserve quoted identifiers.
    pub fn resolve_relation(&self, name: &[&str], context: &SemanticContext) -> RelationResolution {
        let folded = name
            .iter()
            .map(|value| identifier_key(&sqlparser::ast::Ident::new(*value), context.dialect))
            .collect::<Vec<_>>();
        let name = folded.iter().map(String::as_str).collect::<Vec<_>>();
        self.resolve_relation_keys(&name, context)
    }

    // Keys from parsed identifiers have already preserved quotes and folded
    // unquoted Snowflake identifiers. Catalog names are stored verbatim.
    fn resolve_relation_keys(
        &self,
        name: &[&str],
        context: &SemanticContext,
    ) -> RelationResolution {
        let Some(namespace) = relation_namespace(name, context) else {
            return RelationResolution::Unknown;
        };
        let object = match name {
            [object] => *object,
            [_, object] => *object,
            [_, _, object] => *object,
            _ => return RelationResolution::Unknown,
        };
        let matches = self
            .relations()
            .filter(|entry| {
                same_identifier(
                    entry.qualified_name.object.as_str(),
                    object,
                    context.dialect,
                ) && same_optional_identifier(
                    entry.qualified_name.database.as_deref(),
                    namespace.database.as_deref(),
                    context.dialect,
                ) && same_optional_identifier(
                    entry.qualified_name.schema.as_deref(),
                    namespace.schema.as_deref(),
                    context.dialect,
                )
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => RelationResolution::Resolved(entry.id.clone()),
            [] if self.coverage(&namespace).can_prove_missing() => RelationResolution::Missing,
            [] => RelationResolution::Unknown,
            _ => RelationResolution::Ambiguous,
        }
    }
}

fn merge_coverage(left: CatalogCoverage, right: CatalogCoverage) -> CatalogCoverage {
    if left.can_prove_missing() && right.can_prove_missing() {
        CatalogCoverage::Complete
    } else if matches!(left, CatalogCoverage::Failed) || matches!(right, CatalogCoverage::Failed) {
        CatalogCoverage::Failed
    } else if matches!(left, CatalogCoverage::Loading) || matches!(right, CatalogCoverage::Loading)
    {
        CatalogCoverage::Loading
    } else if matches!(left, CatalogCoverage::NotLoaded)
        || matches!(right, CatalogCoverage::NotLoaded)
    {
        CatalogCoverage::NotLoaded
    } else {
        CatalogCoverage::OutOfScope
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationResolution {
    Resolved(CatalogId),
    Missing,
    Ambiguous,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticAnalysis {
    pub diagnostics: Vec<super::SqlDiagnostic>,
    pub dependencies: Vec<CatalogId>,
    pub incomplete: bool,
}

pub fn analyze_semantics(
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
) -> SemanticAnalysis {
    let mut analysis = SemanticAnalysis {
        diagnostics: Vec::new(),
        dependencies: Vec::new(),
        incomplete: false,
    };
    let Ok(statements) =
        sqlparser::parser::Parser::parse_sql(parser_dialect(context.dialect), text)
    else {
        return analysis;
    };
    let index = LineIndex::new(text);
    for statement in statements {
        analyze_statement(&statement, text, context, catalog, &index, &mut analysis);
    }
    analysis
}

fn analyze_statement(
    statement: &sqlparser::ast::Statement,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    match statement {
        sqlparser::ast::Statement::Query(query) => {
            analyze_query(query, text, context, catalog, index, analysis, &[]);
        }
        sqlparser::ast::Statement::Insert(insert) => {
            analyze_insert(insert, text, context, catalog, index, analysis);
        }
        sqlparser::ast::Statement::Update(update) => {
            analyze_update(update, text, context, catalog, index, analysis);
        }
        sqlparser::ast::Statement::Delete(delete) => {
            analyze_delete(delete, text, context, catalog, index, analysis);
        }
        _ => analysis.incomplete = true,
    }
}

fn analyze_insert(
    insert: &sqlparser::ast::Insert,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let sqlparser::ast::TableObject::TableName(name) = &insert.table else {
        analysis.incomplete = true;
        return;
    };
    let Some(relation) = resolve_target_name(name, text, context, catalog, index, analysis) else {
        return;
    };
    for column in &insert.columns {
        validate_relation_column_name(
            column,
            &relation,
            catalog,
            context.dialect,
            text,
            index,
            analysis,
        );
    }
    if let Some(source) = &insert.source {
        analyze_query(source, text, context, catalog, index, analysis, &[]);
    }
}

fn analyze_update(
    update: &sqlparser::ast::Update,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let Some(target) = table_factor_name(&update.table.relation) else {
        analysis.incomplete = true;
        return;
    };
    let Some(relation) = resolve_target_name(target, text, context, catalog, index, analysis)
    else {
        return;
    };
    let source = RelationSource {
        dialect: context.dialect,
        name: target
            .0
            .last()
            .and_then(|part| part.as_ident())
            .map(|ident| identifier_key(ident, context.dialect))
            .unwrap_or_default(),
        relation: Some(relation),
        virtual_columns: Vec::new(),
        virtual_complete: false,
    };
    let sources = [source];
    for assignment in &update.assignments {
        match &assignment.target {
            sqlparser::ast::AssignmentTarget::ColumnName(name) => {
                validate_relation_column_name(
                    name,
                    sources[0].relation.as_ref().unwrap(),
                    catalog,
                    context.dialect,
                    text,
                    index,
                    analysis,
                );
            }
            sqlparser::ast::AssignmentTarget::Tuple(names) => {
                for name in names {
                    validate_relation_column_name(
                        name,
                        sources[0].relation.as_ref().unwrap(),
                        catalog,
                        context.dialect,
                        text,
                        index,
                        analysis,
                    );
                }
            }
        }
        validate_expr(
            &assignment.value,
            &sources,
            &[],
            catalog,
            text,
            index,
            analysis,
        );
    }
    if let Some(selection) = &update.selection {
        validate_expr(selection, &sources, &[], catalog, text, index, analysis);
    }
}

fn analyze_delete(
    delete: &sqlparser::ast::Delete,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let (sqlparser::ast::FromTable::WithFromKeyword(tables)
    | sqlparser::ast::FromTable::WithoutKeyword(tables)) = &delete.from;
    let mut sources = Vec::new();
    for table in tables {
        analyze_table_factor(
            &table.relation,
            context,
            catalog,
            text,
            index,
            &[],
            &mut sources,
            analysis,
        );
        for join in &table.joins {
            analyze_table_factor(
                &join.relation,
                context,
                catalog,
                text,
                index,
                &[],
                &mut sources,
                analysis,
            );
        }
    }
    if let Some(selection) = &delete.selection {
        validate_expr(selection, &sources, &[], catalog, text, index, analysis);
    }
}

fn table_factor_name(factor: &sqlparser::ast::TableFactor) -> Option<&sqlparser::ast::ObjectName> {
    match factor {
        sqlparser::ast::TableFactor::Table { name, .. } => Some(name),
        _ => None,
    }
}

fn resolve_target_name(
    name: &sqlparser::ast::ObjectName,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) -> Option<CatalogId> {
    let parts = name
        .0
        .iter()
        .filter_map(|part| part.as_ident())
        .collect::<Vec<_>>();
    let keys = parts
        .iter()
        .map(|part| identifier_key(part, context.dialect))
        .collect::<Vec<_>>();
    let values = keys.iter().map(String::as_str).collect::<Vec<_>>();
    if !validate_qualified_namespace(&parts, &values, context, catalog, text, index, analysis) {
        return None;
    }
    match catalog.resolve_relation_keys(&values, context) {
        RelationResolution::Resolved(id) => Some(id),
        RelationResolution::Missing => {
            if let Some(last) = parts.last() {
                analysis.diagnostics.push(super::SqlDiagnostic {
                    range: ident_range(text, index, last),
                    message: format!("relation '{}' does not exist", last.value),
                    code: "sql-unknown-relation",
                });
            }
            None
        }
        RelationResolution::Ambiguous => {
            analysis.incomplete = true;
            None
        }
        RelationResolution::Unknown => {
            analysis.incomplete = true;
            None
        }
    }
}

fn validate_qualified_namespace(
    parts: &[&sqlparser::ast::Ident],
    values: &[&str],
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    text: &str,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) -> bool {
    if values.len() == 3
        && if context.dialect == SqlDialect::Snowflake {
            catalog
                .coverage(&CatalogNamespace::new(None::<String>, None::<String>))
                .can_prove_missing()
        } else {
            catalog.databases().next().is_some()
        }
        && matches!(
            context.dialect,
            SqlDialect::Postgres | SqlDialect::SqlServer | SqlDialect::Snowflake
        )
        && !catalog.has_database(values[0], context.dialect)
    {
        analysis.diagnostics.push(super::SqlDiagnostic {
            range: ident_range(text, index, parts[0]),
            message: format!("database '{}' does not exist", parts[0].value),
            code: "sql-unknown-database",
        });
        return false;
    }
    let schema_index = match (context.dialect, values) {
        (SqlDialect::SqlServer | SqlDialect::Snowflake, [_database, _schema, _relation]) => Some(1),
        (_, [_schema, _relation]) => Some(0),
        _ => None,
    };
    if let Some(schema_index) = schema_index {
        let database = if values.len() == 3 {
            Some(values[0])
        } else {
            context.database.as_deref()
        };
        let can_prove_missing = if context.dialect == SqlDialect::Snowflake {
            catalog
                .coverage(&CatalogNamespace::new(database, None::<String>))
                .can_prove_missing()
        } else {
            catalog.schemas().next().is_some()
        };
        if can_prove_missing && !catalog.has_schema(database, values[schema_index], context.dialect)
        {
            analysis.diagnostics.push(super::SqlDiagnostic {
                range: ident_range(text, index, parts[schema_index]),
                message: format!("schema '{}' does not exist", parts[schema_index].value),
                code: "sql-unknown-schema",
            });
            return false;
        }
    }
    true
}

fn validate_relation_column_name(
    name: &sqlparser::ast::ObjectName,
    relation: &CatalogId,
    catalog: &CatalogSnapshot,
    dialect: SqlDialect,
    text: &str,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let Some(column) = name.0.last().and_then(|part| part.as_ident()) else {
        return;
    };
    if catalog.column_coverage(relation) != CatalogCoverage::Complete {
        analysis.incomplete = true;
        return;
    }
    if !catalog.relation_columns(relation).any(|entry| {
        same_identifier(
            &entry.qualified_name.object,
            &identifier_key(column, dialect),
            dialect,
        )
    }) {
        analysis.diagnostics.push(super::SqlDiagnostic {
            range: ident_range(text, index, column),
            message: format!("column '{}' does not exist", column.value),
            code: "sql-unknown-column",
        });
    }
}

fn analyze_query(
    query: &sqlparser::ast::Query,
    text: &str,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
    outer_sources: &[RelationSource],
) {
    let mut local_sources = outer_sources.to_vec();
    if let Some(with) = &query.with {
        if with.recursive {
            analysis.incomplete = true;
        }
        for cte in &with.cte_tables {
            analyze_query(
                &cte.query,
                text,
                context,
                catalog,
                index,
                analysis,
                outer_sources,
            );
            let columns = cte
                .alias
                .columns
                .iter()
                .map(|column| identifier_key(&column.name, context.dialect))
                .collect();
            local_sources.push(RelationSource {
                dialect: context.dialect,
                name: identifier_key(&cte.alias.name, context.dialect),
                relation: None,
                virtual_columns: columns,
                virtual_complete: !cte.alias.columns.is_empty(),
            });
        }
    }
    let sqlparser::ast::SetExpr::Select(select) = query.body.as_ref() else {
        analysis.incomplete = true;
        return;
    };
    let mut sources = Vec::new();
    for table in &select.from {
        analyze_table_factor(
            &table.relation,
            context,
            catalog,
            text,
            index,
            &local_sources,
            &mut sources,
            analysis,
        );
        for join in &table.joins {
            analyze_table_factor(
                &join.relation,
                context,
                catalog,
                text,
                index,
                &local_sources,
                &mut sources,
                analysis,
            );
            if let sqlparser::ast::JoinOperator::Join(constraint)
            | sqlparser::ast::JoinOperator::Inner(constraint)
            | sqlparser::ast::JoinOperator::Left(constraint)
            | sqlparser::ast::JoinOperator::LeftOuter(constraint)
            | sqlparser::ast::JoinOperator::Right(constraint)
            | sqlparser::ast::JoinOperator::RightOuter(constraint)
            | sqlparser::ast::JoinOperator::FullOuter(constraint)
            | sqlparser::ast::JoinOperator::CrossJoin(constraint)
            | sqlparser::ast::JoinOperator::Semi(constraint)
            | sqlparser::ast::JoinOperator::Anti(constraint)
            | sqlparser::ast::JoinOperator::StraightJoin(constraint) = &join.join_operator
                && let sqlparser::ast::JoinConstraint::On(expr) = constraint
            {
                validate_expr(
                    expr,
                    &sources,
                    outer_sources,
                    catalog,
                    text,
                    index,
                    analysis,
                );
            }
            if let sqlparser::ast::JoinOperator::AsOf {
                match_condition,
                constraint: sqlparser::ast::JoinConstraint::On(expr),
            } = &join.join_operator
            {
                validate_expr(
                    match_condition,
                    &sources,
                    outer_sources,
                    catalog,
                    text,
                    index,
                    analysis,
                );
                validate_expr(
                    expr,
                    &sources,
                    outer_sources,
                    catalog,
                    text,
                    index,
                    analysis,
                );
            }
        }
    }
    let output_aliases = select
        .projection
        .iter()
        .filter_map(|item| match item {
            sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => {
                Some(identifier_key(alias, context.dialect))
            }
            sqlparser::ast::SelectItem::ExprWithAliases { aliases, .. } => aliases
                .first()
                .map(|alias| identifier_key(alias, context.dialect)),
            _ => None,
        })
        .collect::<Vec<_>>();
    for item in &select.projection {
        let expr = match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr)
            | sqlparser::ast::SelectItem::ExprWithAlias { expr, .. }
            | sqlparser::ast::SelectItem::ExprWithAliases { expr, .. } => Some(expr),
            sqlparser::ast::SelectItem::QualifiedWildcard(
                sqlparser::ast::SelectItemQualifiedWildcardKind::ObjectName(name),
                _,
            ) => {
                if let Some(qualifier) = name.0.last().and_then(|part| part.as_ident()) {
                    validate_qualifier(qualifier, &sources, text, index, analysis);
                }
                None
            }
            sqlparser::ast::SelectItem::QualifiedWildcard(_, _)
            | sqlparser::ast::SelectItem::Wildcard(_) => None,
        };
        if let Some(expr) = expr {
            validate_expr(
                expr,
                &sources,
                outer_sources,
                catalog,
                text,
                index,
                analysis,
            );
        }
    }
    if let Some(expr) = &select.selection {
        validate_expr(
            expr,
            &sources,
            outer_sources,
            catalog,
            text,
            index,
            analysis,
        );
    }
    if let Some(expr) = &select.having {
        validate_expr(
            expr,
            &sources,
            outer_sources,
            catalog,
            text,
            index,
            analysis,
        );
    }
    if let sqlparser::ast::GroupByExpr::Expressions(expressions, _) = &select.group_by {
        for expr in expressions {
            validate_expr(
                expr,
                &sources,
                outer_sources,
                catalog,
                text,
                index,
                analysis,
            );
        }
    }
    if let Some(order_by) = &query.order_by
        && let sqlparser::ast::OrderByKind::Expressions(expressions) = &order_by.kind
    {
        for order in expressions {
            let is_output_alias = matches!(
                &order.expr,
                sqlparser::ast::Expr::Identifier(identifier)
                    if output_aliases
                        .iter()
                        .any(|alias| same_identifier(alias, &identifier_key(identifier, context.dialect), context.dialect))
            );
            if !is_output_alias {
                validate_expr(
                    &order.expr,
                    &sources,
                    outer_sources,
                    catalog,
                    text,
                    index,
                    analysis,
                );
            }
        }
    }
}

#[derive(Clone)]
struct RelationSource {
    dialect: SqlDialect,
    name: String,
    relation: Option<CatalogId>,
    virtual_columns: Vec<String>,
    virtual_complete: bool,
}

#[allow(clippy::too_many_arguments)]
fn analyze_table_factor(
    factor: &sqlparser::ast::TableFactor,
    context: &SemanticContext,
    catalog: &CatalogSnapshot,
    text: &str,
    index: &LineIndex,
    local_sources: &[RelationSource],
    sources: &mut Vec<RelationSource>,
    analysis: &mut SemanticAnalysis,
) {
    let sqlparser::ast::TableFactor::Table { name, alias, .. } = factor else {
        analysis.incomplete = true;
        return;
    };
    let parts = name
        .0
        .iter()
        .filter_map(|part| part.as_ident())
        .collect::<Vec<_>>();
    let Some(last) = parts.last() else { return };
    let keys = parts
        .iter()
        .map(|part| identifier_key(part, context.dialect))
        .collect::<Vec<_>>();
    let names = keys.iter().map(String::as_str).collect::<Vec<_>>();
    if names.len() == 1
        && let Some(local) = local_sources
            .iter()
            .rev()
            .find(|source| same_identifier(&source.name, names[0], context.dialect))
    {
        sources.push(RelationSource {
            dialect: context.dialect,
            name: identifier_key(
                alias.as_ref().map_or(*last, |alias| &alias.name),
                context.dialect,
            ),
            relation: None,
            virtual_columns: local.virtual_columns.clone(),
            virtual_complete: local.virtual_complete,
        });
        return;
    }
    if !validate_qualified_namespace(&parts, &names, context, catalog, text, index, analysis) {
        return;
    }
    let resolution = catalog.resolve_relation_keys(&names, context);
    let relation = match resolution {
        RelationResolution::Resolved(id) => {
            analysis.dependencies.push(id.clone());
            Some(id)
        }
        RelationResolution::Missing => {
            analysis.diagnostics.push(super::SqlDiagnostic {
                range: ident_range(text, index, last),
                message: format!(
                    "relation '{}' does not exist in the active namespace",
                    last.value
                ),
                code: "sql-unknown-relation",
            });
            None
        }
        RelationResolution::Ambiguous => {
            analysis.diagnostics.push(super::SqlDiagnostic {
                range: ident_range(text, index, last),
                message: format!("relation '{}' is ambiguous", last.value),
                code: "sql-ambiguous-relation",
            });
            None
        }
        RelationResolution::Unknown => {
            analysis.incomplete = true;
            None
        }
    };
    let source_name = identifier_key(
        alias.as_ref().map_or(*last, |alias| &alias.name),
        context.dialect,
    );
    sources.push(RelationSource {
        dialect: context.dialect,
        name: source_name,
        relation,
        virtual_columns: Vec::new(),
        virtual_complete: false,
    });
}

fn validate_expr(
    expr: &sqlparser::ast::Expr,
    sources: &[RelationSource],
    outer_sources: &[RelationSource],
    catalog: &CatalogSnapshot,
    text: &str,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let _ = sqlparser::ast::visit_expressions(expr, |nested| {
        match nested {
            sqlparser::ast::Expr::Identifier(ident) => {
                validate_column(
                    ident,
                    None,
                    sources,
                    outer_sources,
                    catalog,
                    text,
                    index,
                    analysis,
                );
            }
            sqlparser::ast::Expr::CompoundIdentifier(parts) => {
                if let Some((column, qualifier)) = parts.as_slice().split_last() {
                    validate_column(
                        column,
                        qualifier.first(),
                        sources,
                        outer_sources,
                        catalog,
                        text,
                        index,
                        analysis,
                    );
                }
            }
            _ => {}
        }
        ControlFlow::<()>::Continue(())
    });
}

fn validate_qualifier(
    qualifier: &sqlparser::ast::Ident,
    sources: &[RelationSource],
    text: &str,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    if !sources.iter().any(|source| {
        same_identifier(
            &source.name,
            &identifier_key(qualifier, source.dialect),
            source.dialect,
        )
    }) {
        analysis.diagnostics.push(super::SqlDiagnostic {
            range: ident_range(text, index, qualifier),
            message: format!(
                "relation qualifier '{}' cannot be resolved",
                qualifier.value
            ),
            code: "sql-unresolved-qualifier",
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_column(
    column: &sqlparser::ast::Ident,
    qualifier: Option<&sqlparser::ast::Ident>,
    sources: &[RelationSource],
    outer_sources: &[RelationSource],
    catalog: &CatalogSnapshot,
    text: &str,
    index: &LineIndex,
    analysis: &mut SemanticAnalysis,
) {
    let candidates = sources.iter().chain(outer_sources).filter(|source| {
        qualifier.is_none_or(|qualifier| {
            same_identifier(
                &source.name,
                &identifier_key(qualifier, source.dialect),
                source.dialect,
            )
        })
    });
    let mut match_count = 0usize;
    let mut unknown = false;
    for source in candidates {
        if source.relation.is_none() && source.virtual_complete {
            if source.virtual_columns.iter().any(|name| {
                same_identifier(
                    name,
                    &identifier_key(column, source.dialect),
                    source.dialect,
                )
            }) {
                match_count += 1;
            }
            continue;
        }
        let Some(relation) = source.relation.as_ref() else {
            unknown = true;
            continue;
        };
        match catalog.column_coverage(relation) {
            CatalogCoverage::Complete => {
                if catalog.relation_columns(relation).any(|entry| {
                    same_identifier(
                        &entry.qualified_name.object,
                        &identifier_key(column, source.dialect),
                        source.dialect,
                    )
                }) {
                    match_count += 1;
                }
            }
            _ => unknown = true,
        }
    }
    if match_count > 1 {
        analysis.diagnostics.push(super::SqlDiagnostic {
            range: ident_range(text, index, column),
            message: format!("column '{}' is ambiguous", column.value),
            code: "sql-ambiguous-column",
        });
    } else if match_count == 0 && !unknown && !sources.is_empty() {
        analysis.diagnostics.push(super::SqlDiagnostic {
            range: ident_range(text, index, column),
            message: format!("column '{}' does not exist", column.value),
            code: "sql-unknown-column",
        });
    } else if unknown {
        analysis.incomplete = true;
    }
}

fn ident_range(text: &str, index: &LineIndex, ident: &sqlparser::ast::Ident) -> TextRange {
    index.range(text, ident.span.start, ident.span.end)
}

fn relation_namespace(name: &[&str], context: &SemanticContext) -> Option<CatalogNamespace> {
    match (context.dialect, name) {
        (_, [object]) if !object.is_empty() => Some(context.default_namespace()),
        (SqlDialect::MySql, [database, _]) if !database.is_empty() => {
            Some(CatalogNamespace::new(Some(*database), Some(*database)))
        }
        (SqlDialect::Sqlite, [schema, _]) if !schema.is_empty() => Some(CatalogNamespace::new(
            context.database.clone(),
            Some(*schema),
        )),
        (_, [schema, _]) if !schema.is_empty() => Some(CatalogNamespace::new(
            context.database.clone(),
            Some(*schema),
        )),
        (SqlDialect::SqlServer, [database, schema, _]) if !database.is_empty() => {
            Some(CatalogNamespace::new(Some(*database), Some(*schema)))
        }
        (_, [database, schema, _]) if !database.is_empty() => {
            Some(CatalogNamespace::new(Some(*database), Some(*schema)))
        }
        _ => None,
    }
}

fn identifier_key(ident: &sqlparser::ast::Ident, dialect: SqlDialect) -> String {
    if dialect == SqlDialect::Snowflake && ident.quote_style.is_none() {
        ident.value.to_uppercase()
    } else {
        ident.value.clone()
    }
}

fn same_identifier(left: &str, right: &str, dialect: SqlDialect) -> bool {
    if dialect == SqlDialect::Snowflake {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
}

fn same_optional_identifier(left: Option<&str>, right: Option<&str>, dialect: SqlDialect) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_identifier(left, right, dialect),
        (None, None) => true,
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticContext {
    pub dialect: SqlDialect,
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl SemanticContext {
    pub fn new(
        dialect: SqlDialect,
        database: Option<impl Into<String>>,
        schema: Option<impl Into<String>>,
    ) -> Self {
        Self {
            dialect,
            database: database.map(Into::into),
            schema: schema.map(Into::into),
        }
    }

    pub fn default_namespace(&self) -> CatalogNamespace {
        CatalogNamespace {
            database: self.database.clone(),
            schema: self.schema.clone(),
        }
    }
}
