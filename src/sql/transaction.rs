use super::{SqlDialect, SqlRisk, classify_sql, scan_statements};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginRequest {
    Canonical,
}

impl BeginRequest {
    pub const fn canonical_sql(self) -> &'static str {
        "BEGIN"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionControl {
    Begin(BeginRequest),
    Commit,
    Rollback,
    RollbackToSavepoint(String),
    Savepoint(String),
    ReleaseSavepoint(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionSqlError {
    Empty,
    MultipleStatements,
    MixedControlAndData,
    UnsupportedOptions,
    UnsupportedControl,
    InvalidControl,
    RequiresActiveManual,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionSqlClassification {
    Control(TransactionControl),
    Data {
        risk: SqlRisk,
        mysql_implicit_commit: bool,
    },
    Unsupported(TransactionSqlError),
}

pub fn classify_transaction_sql(sql: &str, dialect: SqlDialect) -> TransactionSqlClassification {
    let statements = scan_statements(sql, dialect);
    if statements.is_empty() {
        return TransactionSqlClassification::Unsupported(TransactionSqlError::Empty);
    }
    if statements.len() != 1 {
        let classifications: Vec<_> = statements
            .iter()
            .filter_map(|range| range.get(sql))
            .map(|statement| classify_single(statement, dialect))
            .collect();
        if classifications.iter().any(is_control)
            && classifications.iter().any(|item| !is_control(item))
        {
            return TransactionSqlClassification::Unsupported(
                TransactionSqlError::MixedControlAndData,
            );
        }
        return TransactionSqlClassification::Unsupported(TransactionSqlError::MultipleStatements);
    }
    classify_single(statements[0].get(sql).unwrap_or_default(), dialect)
}

pub fn classify_transaction_batch(
    sql: &str,
    dialect: SqlDialect,
) -> Result<Vec<TransactionControl>, TransactionSqlError> {
    let statements = scan_statements(sql, dialect);
    if statements.is_empty() {
        return Err(TransactionSqlError::Empty);
    }
    let mut controls = Vec::with_capacity(statements.len());
    for range in statements {
        match classify_single(range.get(sql).unwrap_or_default(), dialect) {
            TransactionSqlClassification::Control(control) => controls.push(control),
            TransactionSqlClassification::Data { .. } => {
                return Err(TransactionSqlError::MixedControlAndData);
            }
            TransactionSqlClassification::Unsupported(error) => return Err(error),
        }
    }
    Ok(controls)
}

pub fn savepoint_requires_active_manual(
    control: &TransactionControl,
    mode: crate::model::transaction::TransactionMode,
    state: crate::model::transaction::TransactionState,
) -> bool {
    matches!(
        control,
        TransactionControl::Savepoint(_)
            | TransactionControl::ReleaseSavepoint(_)
            | TransactionControl::RollbackToSavepoint(_)
    ) && !(mode == crate::model::transaction::TransactionMode::Manual
        && state == crate::model::transaction::TransactionState::Active)
}

pub fn validate_transaction_control(
    control: &TransactionControl,
    mode: crate::model::transaction::TransactionMode,
    state: crate::model::transaction::TransactionState,
    read_only: bool,
) -> Result<(), TransactionSqlError> {
    if read_only {
        return Err(TransactionSqlError::ReadOnly);
    }
    if savepoint_requires_active_manual(control, mode, state) {
        return Err(TransactionSqlError::RequiresActiveManual);
    }
    if matches!(control, TransactionControl::Begin(_))
        && state != crate::model::transaction::TransactionState::Idle
    {
        return Err(TransactionSqlError::InvalidControl);
    }
    if matches!(control, TransactionControl::Commit)
        && !(mode == crate::model::transaction::TransactionMode::Manual
            && state == crate::model::transaction::TransactionState::Active)
    {
        return Err(TransactionSqlError::RequiresActiveManual);
    }
    if matches!(control, TransactionControl::Rollback)
        && !(mode == crate::model::transaction::TransactionMode::Manual
            && matches!(
                state,
                crate::model::transaction::TransactionState::Active
                    | crate::model::transaction::TransactionState::Aborted
            ))
    {
        return Err(TransactionSqlError::RequiresActiveManual);
    }
    Ok(())
}

fn classify_single(sql: &str, dialect: SqlDialect) -> TransactionSqlClassification {
    let tokens = tokenize(sql);
    if tokens.is_empty() {
        return TransactionSqlClassification::Unsupported(TransactionSqlError::Empty);
    }
    let upper: Vec<String> = tokens
        .iter()
        .map(|token| token.to_ascii_uppercase())
        .collect();
    let control = match upper.as_slice() {
        [begin] if begin == "BEGIN" => Some(TransactionControl::Begin(BeginRequest::Canonical)),
        [begin, work] if begin == "BEGIN" && work == "WORK" => {
            Some(TransactionControl::Begin(BeginRequest::Canonical))
        }
        [start, transaction] if start == "START" && transaction == "TRANSACTION" => {
            Some(TransactionControl::Begin(BeginRequest::Canonical))
        }
        [commit] if commit == "COMMIT" || commit == "END" => Some(TransactionControl::Commit),
        [rollback] if rollback == "ROLLBACK" => Some(TransactionControl::Rollback),
        [rollback, to, savepoint, _name]
            if rollback == "ROLLBACK" && to == "TO" && savepoint == "SAVEPOINT" =>
        {
            Some(TransactionControl::RollbackToSavepoint(tokens[3].clone()))
        }
        [rollback, to, _name] if rollback == "ROLLBACK" && to == "TO" => {
            Some(TransactionControl::RollbackToSavepoint(tokens[2].clone()))
        }
        [savepoint, _name] if savepoint == "SAVEPOINT" => {
            Some(TransactionControl::Savepoint(tokens[1].clone()))
        }
        [release, savepoint, _name] if release == "RELEASE" && savepoint == "SAVEPOINT" => {
            Some(TransactionControl::ReleaseSavepoint(tokens[2].clone()))
        }
        _ => None,
    };
    if let Some(control) = control {
        return TransactionSqlClassification::Control(control);
    }
    if upper.first().is_some_and(|word| {
        matches!(
            word.as_str(),
            "BEGIN"
                | "START"
                | "COMMIT"
                | "END"
                | "ROLLBACK"
                | "SAVEPOINT"
                | "RELEASE"
                | "SET"
                | "RESET"
        )
    }) {
        return TransactionSqlClassification::Unsupported(
            if upper[0] == "SET" || upper[0] == "RESET" {
                TransactionSqlError::UnsupportedControl
            } else {
                TransactionSqlError::UnsupportedOptions
            },
        );
    }
    let analysis = classify_sql(sql, dialect);
    let risk = analysis.risks.first().copied().unwrap_or(SqlRisk::Unknown);
    TransactionSqlClassification::Data {
        risk,
        mysql_implicit_commit: dialect == SqlDialect::MySql && risk == SqlRisk::Ddl,
    }
}

fn is_control(classification: &TransactionSqlClassification) -> bool {
    matches!(classification, TransactionSqlClassification::Control(_))
}

fn tokenize(sql: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            while chars.next_if(|c| *c != '\n').is_some() {}
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(next) = chars.next() {
                if next == '*' && chars.next_if_eq(&'/').is_some() {
                    break;
                }
            }
        } else if ch == ';' || ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if ch == '\'' || ch == '"' || ch == '`' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            let quote = ch;
            let mut quoted = String::new();
            quoted.push(ch);
            while let Some(next) = chars.next() {
                quoted.push(next);
                if next == quote {
                    if chars.peek() == Some(&quote) {
                        quoted.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
            }
            tokens.push(quoted);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
