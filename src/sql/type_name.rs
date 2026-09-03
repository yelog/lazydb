//! Compact display spellings for database native type names.
//!
//! Postgres reports SQL-standard spellings through `format_type`
//! (`character varying(30)`, `timestamp without time zone`), which are too wide
//! for a completion popup. MySQL, SQL Server and SQLite already report compact
//! names, so any unrecognized input is returned unchanged.

/// Returns a compact display spelling for a database native type name.
///
/// The result is display-only. Callers that need the semantic type (value
/// parsing, DDL generation) must keep using the original name.
pub fn short_type_name(value: &str) -> String {
    let value = value.trim();
    let (head, array) = match value.strip_suffix("[]") {
        Some(head) => (head.trim_end(), "[]"),
        None => (value, ""),
    };
    let (head, zone) = split_time_zone(head);
    let (base, arguments, modifier) = split_parts(head);
    let base = alias(base);

    let mut short = String::with_capacity(value.len());
    if zone == TimeZone::Absent {
        short.push_str(base);
    } else {
        // A matched time-zone phrase means the spelling was rewritten, so emit
        // the canonical lower-case base instead of mixing cases with `tz`.
        short.push_str(&base.to_ascii_lowercase());
    }
    if zone == TimeZone::Aware {
        short.push_str("tz");
    }
    if let Some(arguments) = arguments.filter(|arguments| is_size_arguments(arguments)) {
        short.push('(');
        short.push_str(arguments);
        short.push(')');
    }
    if !modifier.is_empty() {
        short.push(' ');
        short.push_str(modifier);
    }
    short.push_str(array);
    short
}

/// Verbose SQL-standard base names and their compact spellings.
///
/// Matched case-insensitively; unmatched base names keep their original text.
const ALIASES: [(&str, &str); 7] = [
    ("character varying", "varchar"),
    ("character", "char"),
    ("bpchar", "char"),
    ("bit varying", "varbit"),
    ("double precision", "double"),
    ("boolean", "bool"),
    ("integer", "int"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimeZone {
    Absent,
    Naive,
    Aware,
}

/// Splits the trailing `with`/`without time zone` phrase off a type name.
fn split_time_zone(value: &str) -> (&str, TimeZone) {
    for (phrase, zone) in [
        ("without time zone", TimeZone::Naive),
        ("with time zone", TimeZone::Aware),
    ] {
        let Some(split) = value.len().checked_sub(phrase.len()) else {
            continue;
        };
        // `get` rather than `split_at`: the offset is a byte count and type
        // names may end in multi-byte characters.
        if value
            .get(split..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(phrase))
        {
            return (value[..split].trim_end(), zone);
        }
    }
    (value, TimeZone::Absent)
}

/// Splits a type name into base name, parenthesized arguments and any trailing
/// modifier (`bigint(20) unsigned`).
fn split_parts(value: &str) -> (&str, Option<&str>, &str) {
    let Some(open) = value.find('(') else {
        return (value, None, "");
    };
    // `rfind`, so parentheses inside quoted values do not split the name.
    let Some(close) = value.rfind(')').filter(|close| *close > open) else {
        return (value, None, "");
    };
    (
        value[..open].trim_end(),
        Some(&value[open + 1..close]),
        value[close + 1..].trim(),
    )
}

fn alias(base: &str) -> &str {
    ALIASES
        .iter()
        .find(|(verbose, _)| base.eq_ignore_ascii_case(verbose))
        .map_or(base, |(_, short)| *short)
}

/// Precision/length arguments carry information worth showing; value lists
/// (`enum('a','b')`) do not.
fn is_size_arguments(arguments: &str) -> bool {
    arguments
        .chars()
        .any(|character| character.is_ascii_digit())
        && arguments
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, ',' | ' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_standard_spellings_are_shortened() {
        for (native, expected) in [
            ("character varying(30)", "varchar(30)"),
            ("character varying", "varchar"),
            ("character(4)", "char(4)"),
            ("bpchar", "char"),
            ("bit varying(8)", "varbit(8)"),
            ("double precision", "double"),
            ("boolean", "bool"),
            ("integer", "int"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn time_zone_modifiers_are_folded_into_the_base_name() {
        for (native, expected) in [
            ("timestamp without time zone", "timestamp"),
            ("timestamp with time zone", "timestamptz"),
            ("timestamp(3) without time zone", "timestamp(3)"),
            ("timestamp(3) with time zone", "timestamptz(3)"),
            ("time without time zone", "time"),
            ("time with time zone", "timetz"),
            ("TIMESTAMP WITHOUT TIME ZONE", "timestamp"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn size_arguments_are_kept_and_value_lists_are_dropped() {
        for (native, expected) in [
            ("numeric(10,2)", "numeric(10,2)"),
            ("numeric(10, 2)", "numeric(10, 2)"),
            ("bigint(20) unsigned", "bigint(20) unsigned"),
            ("enum('active','pending')", "enum"),
            ("enum('a(1)','b')", "enum"),
            ("set('a','b')", "set"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn array_suffix_and_unknown_modifiers_survive() {
        for (native, expected) in [
            ("character varying(30)[]", "varchar(30)[]"),
            ("text[]", "text[]"),
            ("interval day to second(6)", "interval day to second(6)"),
            ("int unsigned", "int unsigned"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn unrecognized_names_are_returned_verbatim() {
        for native in [
            "bigint",
            "jsonb",
            "uuid",
            "nvarchar",
            "datetime2",
            "extensions.citext",
            // Non-ASCII tail whose byte length would land a time-zone phrase
            // offset inside a character.
            "abc\u{4e2d}xxxxxxxxxxxxx",
            "text\u{1b}[31m",
            "TEXT",
        ] {
            assert_eq!(short_type_name(native), native, "native: {native}");
        }
    }
}
