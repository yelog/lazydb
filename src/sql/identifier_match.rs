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
    identifier_match_details(candidate, query).map(|(kind, _)| kind)
}

#[allow(dead_code)]
pub(crate) fn identifier_match_positions(candidate: &str, query: &str) -> Option<Vec<usize>> {
    identifier_match_details(candidate, query).map(|(_, positions)| positions)
}

fn identifier_match_details(candidate: &str, query: &str) -> Option<(IdentifierMatch, Vec<usize>)> {
    let folded_candidate = folded_positions(candidate);
    let folded_query: Vec<char> = fold_identifier(query).chars().collect();
    let compact_query: Vec<char> = folded_query
        .iter()
        .copied()
        .filter(|character| !is_compact_separator(*character))
        .collect();

    let (kind, positions) = if folded_candidate
        .iter()
        .map(|(character, _)| *character)
        .eq(folded_query.iter().copied())
    {
        (
            IdentifierMatch::Exact,
            folded_candidate
                .iter()
                .take(folded_query.len())
                .map(|(_, position)| *position)
                .collect(),
        )
    } else if folded_candidate.len() >= folded_query.len()
        && folded_candidate
            .iter()
            .map(|(character, _)| *character)
            .take(folded_query.len())
            .eq(folded_query.iter().copied())
    {
        (
            IdentifierMatch::Prefix,
            folded_candidate
                .iter()
                .take(folded_query.len())
                .map(|(_, position)| *position)
                .collect(),
        )
    } else {
        if compact_query.is_empty() {
            return None;
        }
        let compact_candidate: Vec<(char, usize)> = folded_candidate
            .iter()
            .copied()
            .filter(|(character, _)| !is_compact_separator(*character))
            .collect();
        if compact_candidate.len() < compact_query.len()
            || !compact_candidate
                .iter()
                .map(|(character, _)| character)
                .take(compact_query.len())
                .eq(compact_query.iter())
        {
            return None;
        }
        (
            IdentifierMatch::CompactPrefix,
            compact_candidate
                .iter()
                .take(compact_query.len())
                .map(|(_, position)| *position)
                .collect(),
        )
    };

    Some((kind, positions))
}

fn is_compact_separator(character: char) -> bool {
    character == '_' || character == '-' || character.is_ascii_whitespace()
}

fn folded_positions(value: &str) -> Vec<(char, usize)> {
    value
        .char_indices()
        .flat_map(|(position, character)| {
            character
                .to_lowercase()
                .map(move |folded| (folded, position))
        })
        .collect()
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

    #[test]
    fn reports_candidate_byte_positions_for_each_match_kind() {
        assert_eq!(
            identifier_match_positions("sys_user", "sys_"),
            Some(vec![0, 1, 2, 3])
        );
        assert_eq!(
            identifier_match_positions("sys_user", "sysuser"),
            Some(vec![0, 1, 2, 4, 5, 6, 7])
        );
        assert_eq!(
            identifier_match_positions("SYS", "sys"),
            Some(vec![0, 1, 2])
        );
    }

    #[test]
    fn position_matching_preserves_identifier_match_rejections() {
        assert_eq!(identifier_match_positions("sys$user", "sysuser"), None);
        assert_eq!(identifier_match_positions("sys_user", "_"), None);
    }
}
