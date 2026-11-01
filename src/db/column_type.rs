use crate::profile::DatabaseKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnTypeIssueKind {
    Invalid,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnTypeIssue {
    pub kind: ColumnTypeIssueKind,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColumnTypePolicy {
    kind: DatabaseKind,
}

impl ColumnTypePolicy {
    pub const fn for_database(kind: DatabaseKind) -> Self {
        Self { kind }
    }

    pub const fn database_kind(self) -> DatabaseKind {
        self.kind
    }

    pub const fn default_native_type(self) -> &'static str {
        match self.kind {
            DatabaseKind::Oracle => "VARCHAR2(255 CHAR)",
            DatabaseKind::Postgres | DatabaseKind::CockroachDb | DatabaseKind::Sqlite => "text",
            DatabaseKind::DuckDb | DatabaseKind::Snowflake => "VARCHAR",
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb => "text",
            DatabaseKind::SqlServer => "nvarchar(255)",
            DatabaseKind::Redis
            | DatabaseKind::Valkey
            | DatabaseKind::MongoDb
            | DatabaseKind::Elasticsearch => "text",
            DatabaseKind::ClickHouse => "String",
        }
    }

    pub fn validate_native_type(self, value: &str) -> Option<ColumnTypeIssue> {
        if self.kind != DatabaseKind::Oracle {
            return None;
        }
        validate_oracle_type(value)
    }
}

fn validate_oracle_type(value: &str) -> Option<ColumnTypeIssue> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(ColumnTypeIssue {
            kind: ColumnTypeIssueKind::Invalid,
            message: "column type is required".into(),
            suggestion: None,
        });
    }

    let (base, arguments) = split_type_declaration(trimmed);
    if base.eq_ignore_ascii_case("text") {
        return Some(ColumnTypeIssue {
            kind: ColumnTypeIssueKind::Invalid,
            message: "TEXT is not an Oracle datatype".into(),
            suggestion: Some("Use VARCHAR2(n CHAR) for ordinary text or CLOB for long text".into()),
        });
    }

    let Some(arguments) = arguments else {
        return match base.to_ascii_uppercase().as_str() {
            "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" | "RAW" => Some(ColumnTypeIssue {
                kind: ColumnTypeIssueKind::Invalid,
                message: format!("{base} requires a length"),
                suggestion: Some(format!("Use {base}(n)")),
            }),
            "NUMBER"
            | "DATE"
            | "CLOB"
            | "BLOB"
            | "JSON"
            | "XMLTYPE"
            | "FLOAT"
            | "BINARY_FLOAT"
            | "BINARY_DOUBLE"
            | "LONG"
            | "LONG RAW"
            | "ROWID"
            | "UROWID"
            | "INTERVAL DAY TO SECOND"
            | "INTERVAL YEAR TO MONTH" => None,
            _ => Some(ColumnTypeIssue {
                kind: ColumnTypeIssueKind::Unknown,
                message: format!("Oracle datatype {base} was not validated locally"),
                suggestion: None,
            }),
        };
    };

    let arguments = arguments.trim();
    if arguments.is_empty()
        || !arguments.chars().all(|character| {
            character.is_ascii_digit()
                || matches!(character, ',' | ' ' | '+' | '-' | '*' | '_')
                || character.is_ascii_alphabetic()
        })
    {
        return Some(ColumnTypeIssue {
            kind: ColumnTypeIssueKind::Invalid,
            message: format!("invalid parameters for Oracle datatype {base}"),
            suggestion: None,
        });
    }

    if matches!(
        base.to_ascii_uppercase().as_str(),
        "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" | "RAW"
    ) && arguments.split_whitespace().next().is_none_or(|length| {
        length == "0" || !length.chars().all(|character| character.is_ascii_digit())
    }) {
        return Some(ColumnTypeIssue {
            kind: ColumnTypeIssueKind::Invalid,
            message: format!("{base} length must be a positive integer"),
            suggestion: None,
        });
    }
    None
}

fn split_type_declaration(value: &str) -> (&str, Option<&str>) {
    let Some(open) = value.find('(') else {
        return (value, None);
    };
    let base = value[..open].trim();
    let Some(close) = value.rfind(')') else {
        return (base, Some(&value[open + 1..]));
    };
    (base, Some(&value[open + 1..close]))
}

#[cfg(test)]
mod tests {
    use super::{ColumnTypeIssueKind, ColumnTypePolicy};
    use crate::profile::DatabaseKind;

    fn oracle() -> ColumnTypePolicy {
        ColumnTypePolicy::for_database(DatabaseKind::Oracle)
    }

    #[test]
    fn oracle_uses_a_character_semantics_string_default() {
        assert_eq!(oracle().default_native_type(), "VARCHAR2(255 CHAR)");
    }

    #[test]
    fn oracle_rejects_text_with_a_useful_suggestion() {
        let issue = oracle().validate_native_type(" text ").unwrap();
        assert_eq!(issue.kind, ColumnTypeIssueKind::Invalid);
        assert!(issue.message.contains("not an Oracle datatype"));
        assert!(issue.suggestion.unwrap().contains("VARCHAR2"));
    }

    #[test]
    fn oracle_accepts_common_parameterized_types() {
        for value in [
            "VARCHAR2(20 CHAR)",
            "VARCHAR2(20 BYTE)",
            "NUMBER",
            "NUMBER(18,2)",
            "NUMBER(5,-2)",
            "DATE",
            "CLOB",
        ] {
            assert!(oracle().validate_native_type(value).is_none(), "{value}");
        }
    }

    #[test]
    fn oracle_rejects_missing_or_zero_string_lengths() {
        for value in ["VARCHAR2", "VARCHAR2(0)", "NUMBER("] {
            assert_eq!(
                oracle().validate_native_type(value).unwrap().kind,
                ColumnTypeIssueKind::Invalid,
                "{value}"
            );
        }
    }

    #[test]
    fn unknown_types_are_not_rejected_as_known_invalid_types() {
        assert_eq!(
            oracle()
                .validate_native_type("APP.CUSTOM_TYPE")
                .unwrap()
                .kind,
            ColumnTypeIssueKind::Unknown
        );
    }
}
