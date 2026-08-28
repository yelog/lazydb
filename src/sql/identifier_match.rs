#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum IdentifierMatch {
    CompactPrefix,
    Prefix,
    Exact,
}

pub(crate) fn fold_identifier(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

pub(crate) fn compact_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character != '_' && *character != '-' && !character.is_ascii_whitespace()
        })
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn identifier_match(candidate: &str, query: &str) -> Option<IdentifierMatch> {
    let candidate = fold_identifier(candidate);
    let query = fold_identifier(query);
    if candidate == query {
        Some(IdentifierMatch::Exact)
    } else if candidate.starts_with(&query) {
        Some(IdentifierMatch::Prefix)
    } else if let compact_query = compact_identifier(&query)
        && !compact_query.is_empty()
        && compact_identifier(&candidate).starts_with(&compact_query)
    {
        Some(IdentifierMatch::CompactPrefix)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_exact_prefix_and_compact_prefix_matches() {
        assert_eq!(
            identifier_match("sys_user", "SYS_USER"),
            Some(IdentifierMatch::Exact)
        );
        assert_eq!(
            identifier_match("sys_user", "sys_"),
            Some(IdentifierMatch::Prefix)
        );
        assert_eq!(
            identifier_match("sys_user", "sysu"),
            Some(IdentifierMatch::CompactPrefix)
        );
        assert_eq!(
            identifier_match("sys_user", "sysuser"),
            Some(IdentifierMatch::CompactPrefix)
        );
        assert_eq!(
            identifier_match("sys-user", "sysuser"),
            Some(IdentifierMatch::CompactPrefix)
        );
        assert_eq!(
            identifier_match("sys user", "sysuser"),
            Some(IdentifierMatch::CompactPrefix)
        );
    }

    #[test]
    fn rejects_subsequences_suffixes_and_non_separator_compaction() {
        assert_eq!(identifier_match("sys_user", "syusr"), None);
        assert_eq!(identifier_match("sys_user", "user"), None);
        assert_eq!(identifier_match("sys$user", "sysuser"), None);
        assert_eq!(identifier_match("sys.user", "sysuser"), None);
        assert_eq!(identifier_match("sys_user", "_"), None);
    }
}
