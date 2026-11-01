use super::{CompletionKind, SqlDialect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Builtin {
    pub name: &'static str,
    pub kind: CompletionKind,
    pub detail: &'static str,
}

const CURRENT_DATE: Builtin = Builtin {
    name: "CURRENT_DATE",
    kind: CompletionKind::BuiltinExpression,
    detail: "built-in expression",
};

const CURRENT_TIME: Builtin = Builtin {
    name: "CURRENT_TIME",
    kind: CompletionKind::BuiltinExpression,
    detail: "built-in expression",
};

const CURRENT_TIMESTAMP: Builtin = Builtin {
    name: "CURRENT_TIMESTAMP",
    kind: CompletionKind::BuiltinExpression,
    detail: "built-in expression",
};

const CURRENT_EXPRESSIONS: &[Builtin] = &[CURRENT_DATE, CURRENT_TIME, CURRENT_TIMESTAMP];

const COMMON: &[Builtin] = &[
    CURRENT_DATE,
    CURRENT_TIME,
    CURRENT_TIMESTAMP,
    Builtin {
        name: "COALESCE",
        kind: CompletionKind::Function,
        detail: "COALESCE(...)",
    },
    Builtin {
        name: "NULLIF",
        kind: CompletionKind::Function,
        detail: "NULLIF(...)",
    },
    Builtin {
        name: "LOWER",
        kind: CompletionKind::Function,
        detail: "LOWER(...)",
    },
    Builtin {
        name: "UPPER",
        kind: CompletionKind::Function,
        detail: "UPPER(...)",
    },
    Builtin {
        name: "ABS",
        kind: CompletionKind::Function,
        detail: "ABS(...)",
    },
];

const POSTGRES: &[Builtin] = &[Builtin {
    name: "NOW",
    kind: CompletionKind::Function,
    detail: "NOW()",
}];

const MYSQL: &[Builtin] = &[
    Builtin {
        name: "NOW",
        kind: CompletionKind::Function,
        detail: "NOW()",
    },
    Builtin {
        name: "IFNULL",
        kind: CompletionKind::Function,
        detail: "IFNULL(...)",
    },
    Builtin {
        name: "CHAR_LENGTH",
        kind: CompletionKind::Function,
        detail: "CHAR_LENGTH(...)",
    },
];

const SQL_SERVER: &[Builtin] = &[
    Builtin {
        name: "GETDATE",
        kind: CompletionKind::Function,
        detail: "GETDATE()",
    },
    Builtin {
        name: "SYSDATETIME",
        kind: CompletionKind::Function,
        detail: "SYSDATETIME()",
    },
    Builtin {
        name: "LEN",
        kind: CompletionKind::Function,
        detail: "LEN(...)",
    },
];

const SQLITE: &[Builtin] = &[
    Builtin {
        name: "IFNULL",
        kind: CompletionKind::Function,
        detail: "IFNULL(...)",
    },
    Builtin {
        name: "LENGTH",
        kind: CompletionKind::Function,
        detail: "LENGTH(...)",
    },
    Builtin {
        name: "DATETIME",
        kind: CompletionKind::Function,
        detail: "DATETIME(...)",
    },
    Builtin {
        name: "STRFTIME",
        kind: CompletionKind::Function,
        detail: "STRFTIME(...)",
    },
];

const CLICKHOUSE: &[Builtin] = &[
    Builtin {
        name: "arrayJoin",
        kind: CompletionKind::Function,
        detail: "arrayJoin(...)",
    },
    Builtin {
        name: "count",
        kind: CompletionKind::Function,
        detail: "count()",
    },
    Builtin {
        name: "countIf",
        kind: CompletionKind::Function,
        detail: "countIf(condition)",
    },
    Builtin {
        name: "dictGet",
        kind: CompletionKind::Function,
        detail: "dictGet(...)",
    },
    Builtin {
        name: "if",
        kind: CompletionKind::Function,
        detail: "if(condition, then, else)",
    },
    Builtin {
        name: "length",
        kind: CompletionKind::Function,
        detail: "length(...)",
    },
    Builtin {
        name: "now",
        kind: CompletionKind::Function,
        detail: "now()",
    },
    Builtin {
        name: "toDate",
        kind: CompletionKind::Function,
        detail: "toDate(value)",
    },
    Builtin {
        name: "toDateTime",
        kind: CompletionKind::Function,
        detail: "toDateTime(value)",
    },
    Builtin {
        name: "toString",
        kind: CompletionKind::Function,
        detail: "toString(value)",
    },
    Builtin {
        name: "uniqExact",
        kind: CompletionKind::Function,
        detail: "uniqExact(...)",
    },
];

pub(super) fn expression_builtins(dialect: SqlDialect) -> impl Iterator<Item = Builtin> {
    COMMON
        .iter()
        .copied()
        .chain(dialect_builtins(dialect).iter().copied())
}

pub(super) fn default_value_builtins(dialect: SqlDialect) -> impl Iterator<Item = Builtin> {
    CURRENT_EXPRESSIONS
        .iter()
        .copied()
        .chain(default_value_dialect_builtins(dialect).iter().copied())
}

fn dialect_builtins(dialect: SqlDialect) -> &'static [Builtin] {
    match dialect {
        SqlDialect::Postgres => POSTGRES,
        SqlDialect::MySql | SqlDialect::MariaDb => MYSQL,
        SqlDialect::SqlServer => SQL_SERVER,
        SqlDialect::Sqlite => SQLITE,
        SqlDialect::ClickHouse => CLICKHOUSE,
        SqlDialect::DuckDb => &[],
        SqlDialect::Generic | SqlDialect::Oracle | SqlDialect::Snowflake => &[],
    }
}

fn default_value_dialect_builtins(dialect: SqlDialect) -> &'static [Builtin] {
    match dialect {
        SqlDialect::Postgres => &[Builtin {
            name: "NOW",
            kind: CompletionKind::Function,
            detail: "NOW()",
        }],
        SqlDialect::SqlServer => &[
            Builtin {
                name: "GETDATE",
                kind: CompletionKind::Function,
                detail: "GETDATE()",
            },
            Builtin {
                name: "SYSDATETIME",
                kind: CompletionKind::Function,
                detail: "SYSDATETIME()",
            },
        ],
        SqlDialect::MySql
        | SqlDialect::MariaDb
        | SqlDialect::Sqlite
        | SqlDialect::DuckDb
        | SqlDialect::Generic
        | SqlDialect::Oracle
        | SqlDialect::ClickHouse
        | SqlDialect::Snowflake => &[],
    }
}
