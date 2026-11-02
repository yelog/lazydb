//! Compatibility for GoogleSQL lexical forms missing in sqlparser 0.62.
//! This only builds parser tokens; execution always uses the original SQL.
use sqlparser::{
    dialect::BigQueryDialect,
    parser::{Parser, ParserError},
    tokenizer::{Token, TokenWithSpan, Tokenizer, TokenizerError},
};

pub(super) fn tokenize(sql: &str) -> Result<Vec<TokenWithSpan>, TokenizerError> {
    let mut masked = sql.to_owned();
    let mut covered = 0;
    loop {
        let mut tokens = Vec::new();
        let result =
            Tokenizer::new(&BigQueryDialect, &masked).tokenize_with_location_into_buf(&mut tokens);
        let lines = super::LineIndex::new(sql);
        let quoted = tokens.iter().find_map(|token| {
            let Token::Word(word) = &token.token else {
                return None;
            };
            let start = lines.offset(sql, token.span.start.line, token.span.start.column);
            (word.quote_style == Some('`') && start >= covered).then_some(start)
        });
        let Some(start) = quoted else {
            return result.and_then(|_| {
                for token in &mut tokens {
                    if let Token::Word(word) = &mut token.token
                        && word.quote_style == Some('`')
                    {
                        let start =
                            lines.offset(sql, token.span.start.line, token.span.start.column);
                        let end = lines.offset(sql, token.span.end.line, token.span.end.column);
                        word.value =
                            unescape_identifier(&sql[start + 1..end - 1]).ok_or_else(|| {
                                TokenizerError {
                                    message: "Invalid GoogleSQL quoted identifier escape".into(),
                                    location: token.span.start,
                                }
                            })?;
                    }
                }
                Ok(tokens)
            });
        };
        let bytes = sql.as_bytes();
        let mut end = start + 1;
        let mut escapes = Vec::new();
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                if let Some(next) = sql.get(end + 1..).and_then(|suffix| suffix.chars().next()) {
                    if matches!(next, '\n' | '\r') {
                        break;
                    }
                    escapes.push((end, end + 1 + next.len_utf8()));
                    end += 1 + next.len_utf8();
                    continue;
                }
                break;
            }
            if bytes[end] == b'`' {
                break;
            }
            end += 1;
        }
        if bytes.get(end) != Some(&b'`') {
            // An escaped delimiter must not turn an unfinished path into code.
            return Err(TokenizerError {
                message: "Expected close delimiter '`' before EOF.".into(),
                location: tokens
                    .iter()
                    .find(|token| {
                        lines.offset(sql, token.span.start.line, token.span.start.column) == start
                    })
                    .expect("quoted token exists")
                    .span
                    .start,
            });
        }
        for (start, end) in escapes {
            // Equal character width preserves locations (including Unicode).
            let replacement = "_".repeat(sql[start..end].chars().count());
            // Work on character locations because byte widths can change.
            let prefix_chars = sql[..start].chars().count();
            let masked_start = masked
                .char_indices()
                .nth(prefix_chars)
                .map(|(offset, _)| offset)
                .unwrap_or(masked.len());
            let masked_end = masked[masked_start..]
                .char_indices()
                .nth(replacement.len())
                .map(|(offset, _)| masked_start + offset)
                .unwrap_or(masked.len());
            masked.replace_range(masked_start..masked_end, &replacement);
        }
        covered = end + 1;
    }
}

fn unescape_identifier(source: &str) -> Option<String> {
    let mut chars = source.chars();
    let mut value = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        let decoded = match escaped {
            'a' => '\u{7}',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{b}',
            'x' | 'u' | 'U' => {
                let count = match escaped {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let mut code = 0u32;
                for _ in 0..count {
                    code = code
                        .checked_mul(16)?
                        .checked_add(chars.next()?.to_digit(16)?)?;
                }
                char::from_u32(code)?
            }
            '0'..='7' => {
                let mut code = escaped.to_digit(8)?;
                for _ in 0..2 {
                    code = code * 8 + chars.next()?.to_digit(8)?;
                }
                char::from_u32(code)?
            }
            other => other,
        };
        value.push(decoded);
    }
    Some(value)
}

pub(super) fn parse(sql: &str) -> Result<Vec<sqlparser::ast::Statement>, ParserError> {
    let tokens = tokenize(sql).map_err(|error| ParserError::TokenizerError(error.to_string()))?;
    let mut compatible = Vec::with_capacity(tokens.len());
    let mut type_depth: usize = 0;
    let mut previous_type = false;
    for token in tokens {
        if matches!(token.token, Token::Whitespace(_)) {
            compatible.push(token);
            continue;
        }
        match &token.token {
            Token::Lt if previous_type || type_depth > 0 => type_depth += 1,
            Token::Gt if type_depth > 0 => type_depth -= 1,
            Token::ShiftRight if type_depth >= 2 => {
                type_depth -= 2;
                compatible.push(TokenWithSpan::new(Token::Gt, token.span));
                compatible.push(TokenWithSpan::new(Token::Gt, token.span));
                previous_type = false;
                continue;
            }
            _ => {}
        }
        previous_type = matches!(&token.token, Token::Word(word) if word.quote_style.is_none() && matches!(word.value.to_ascii_uppercase().as_str(), "ARRAY" | "STRUCT" | "RANGE"));
        compatible.push(token);
    }
    let mut normalized = Vec::new();
    let mut position = 0;
    while position < compatible.len() {
        if matches!(&compatible[position].token, Token::Word(word) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("ARRAY"))
        {
            let mut cursor = position + 1;
            while compatible
                .get(cursor)
                .is_some_and(|token| matches!(token.token, Token::Whitespace(_)))
            {
                cursor += 1;
            }
            if compatible
                .get(cursor)
                .is_some_and(|token| token.token == Token::Lt)
            {
                let mut depth = 0;
                let mut end = cursor;
                while end < compatible.len() {
                    match compatible[end].token {
                        Token::Lt => depth += 1,
                        Token::Gt => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                let mut next = end + 1;
                while compatible
                    .get(next)
                    .is_some_and(|token| matches!(token.token, Token::Whitespace(_)))
                {
                    next += 1;
                }
                if compatible
                    .get(next)
                    .is_some_and(|token| token.token == Token::LBracket)
                {
                    // Validate the complete type with the official type grammar,
                    // then omit only its annotation from the risk AST. Literal
                    // elements (including subqueries/functions) remain visited.
                    Parser::new(&BigQueryDialect)
                        .with_tokens_with_locations(compatible[position..=end].to_vec())
                        .parse_data_type()?;
                    position = next;
                    continue;
                }
            }
        }
        normalized.push(compatible[position].clone());
        position += 1;
    }
    Parser::new(&BigQueryDialect)
        .with_tokens_with_locations(normalized)
        .parse_statements()
}

#[cfg(test)]
mod tests {
    #[test]
    fn quoted_identifier_tokens_retain_logical_escape_values() {
        for (source, expected) in [
            (r"`odd\`name`", "odd`name"),
            (r"`odd\\name`", "odd\\name"),
            (r"`café`", "café"),
            (r"`\u0061`", "a"),
            (r"`\U0001F600`", "\u{1f600}"),
            (r"`\x61\141`", "aa"),
            (r"`odd\`名`", "odd`名"),
        ] {
            let tokens = super::tokenize(source).unwrap();
            assert!(
                matches!(&tokens[0].token, sqlparser::tokenizer::Token::Word(word) if word.value == expected)
            );
        }
        for source in [
            r"`\x`",
            r"`\x6`",
            r"`\xGG`",
            r"`\u`",
            r"`\u061`",
            r"`\u00GG`",
            r"`\uD800`",
            r"`\U0001F60`",
            r"`\U0001F60G`",
            r"`\U00110000`",
            r"`\UFFFFFFFF`",
            r"`\1`",
            r"`\14`",
            r"`\148`",
        ] {
            assert!(super::tokenize(source).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn typed_struct_array_parser_probe() {
        super::parse("SELECT ARRAY<STRUCT<x INT64>>[STRUCT(1)]").unwrap();
    }
}
