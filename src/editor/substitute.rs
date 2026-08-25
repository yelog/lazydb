//! Vim-style `:s` planning using Rust's `regex` syntax and semantics.
//!
//! Ex ranges and replacement conveniences follow Vim where practical. Pattern
//! matching itself is Rust regex: backreferences and look-around are not
//! supported, while capture groups use `$1`-style expansion and `&` means the
//! whole match.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LineRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubstitutionMatch {
    pub range: Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubstitutionPlan {
    pub source: String,
    pub matches: Vec<SubstitutionMatch>,
    pub pattern: String,
    pub replacement: String,
    pub range: LineRange,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SubstituteError {
    #[error("invalid substitute command")]
    Syntax,
    #[error("invalid regex: {0}")]
    Regex(String),
    #[error("pattern not found")]
    NoMatch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Flags {
    global: bool,
    case_sensitive: Option<bool>,
    pub(crate) confirm: bool,
}

pub(crate) fn plan(
    source: &str,
    command: &str,
    cursor_line: usize,
    visual: Option<LineRange>,
    previous_pattern: Option<&str>,
) -> Result<(SubstitutionPlan, Flags), SubstituteError> {
    let command = command.trim();
    let (range, body) = parse_range(command, cursor_line, visual)?;
    let body = body.strip_prefix('s').ok_or(SubstituteError::Syntax)?;
    let mut chars = body.chars();
    let delimiter = chars.next().ok_or(SubstituteError::Syntax)?;
    if delimiter.is_ascii_alphanumeric() || delimiter.is_whitespace() {
        return Err(SubstituteError::Syntax);
    }
    let rest = chars.as_str();
    let (pattern, rest) = field(rest, delimiter)?;
    let (replacement, flags_text) = field(rest, delimiter)?;
    let mut flags = Flags::default();
    for flag in flags_text.chars() {
        match flag {
            'g' => flags.global = true,
            'i' => flags.case_sensitive = Some(false),
            'I' => flags.case_sensitive = Some(true),
            'c' => flags.confirm = true,
            _ => return Err(SubstituteError::Syntax),
        }
    }
    let pattern = if pattern.is_empty() {
        previous_pattern.ok_or(SubstituteError::Syntax)?.to_owned()
    } else {
        pattern
    };
    let mut builder = RegexBuilder::new(&pattern);
    if flags.case_sensitive == Some(false) {
        builder.case_insensitive(true);
    }
    let regex = builder
        .build()
        .map_err(|error| SubstituteError::Regex(error.to_string()))?;
    let lines = line_offsets(source);
    let range = LineRange {
        start: range.start.min(lines.len().saturating_sub(1)),
        end: range.end.min(lines.len().saturating_sub(1)),
    };
    let mut matches = Vec::new();
    for line in range.start..=range.end {
        let start = lines[line];
        let end = lines.get(line + 1).copied().unwrap_or(source.len());
        let end = end.saturating_sub(usize::from(
            source.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n'),
        ));
        let line_text = &source[start..end];
        let iter = if flags.global {
            regex.find_iter(line_text).collect::<Vec<_>>()
        } else {
            regex.find_iter(line_text).take(1).collect::<Vec<_>>()
        };
        for found in iter {
            let replacement = expand_replacement(&replacement, &regex, line_text, found.range());
            matches.push(SubstitutionMatch {
                range: (start + found.start())..(start + found.end()),
                replacement,
            });
        }
    }
    if matches.is_empty() {
        return Err(SubstituteError::NoMatch);
    }
    Ok((
        SubstitutionPlan {
            source: source.to_owned(),
            matches,
            pattern,
            replacement,
            range,
        },
        flags,
    ))
}

fn parse_range(
    command: &str,
    cursor_line: usize,
    visual: Option<LineRange>,
) -> Result<(LineRange, &str), SubstituteError> {
    if let Some(body) = command.strip_prefix("%") {
        return Ok((
            LineRange {
                start: 0,
                end: usize::MAX,
            },
            body,
        ));
    }
    if let Some(body) = command.strip_prefix("'<,'>") {
        return Ok((visual.ok_or(SubstituteError::Syntax)?, body));
    }
    if let Some((range, _)) = command.split_once('s') {
        if range.is_empty() {
            return Ok((
                LineRange {
                    start: cursor_line,
                    end: cursor_line,
                },
                command,
            ));
        }
        let mut values = range.split(',');
        let start = parse_address(values.next().ok_or(SubstituteError::Syntax)?, cursor_line)?;
        let end = values
            .next()
            .map(|value| parse_address(value, cursor_line))
            .transpose()?
            .unwrap_or(start);
        if values.next().is_some() || start > end {
            return Err(SubstituteError::Syntax);
        }
        return Ok((LineRange { start, end }, &command[range.len()..]));
    }
    Err(SubstituteError::Syntax)
}

fn parse_address(value: &str, cursor_line: usize) -> Result<usize, SubstituteError> {
    match value {
        "." => Ok(cursor_line),
        "$" => Ok(usize::MAX),
        value => value
            .parse()
            .ok()
            .and_then(|line: usize| line.checked_sub(1))
            .ok_or(SubstituteError::Syntax),
    }
}

fn field(input: &str, delimiter: char) -> Result<(String, &str), SubstituteError> {
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == delimiter {
            return Ok((
                unescape(&input[..index], delimiter),
                &input[index + delimiter.len_utf8()..],
            ));
        }
    }
    Err(SubstituteError::Syntax)
}

fn unescape(value: &str, delimiter: char) -> String {
    let mut output = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            if character != delimiter && character != '\\' {
                output.push('\\');
            }
            output.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    if escaped {
        output.push('\\');
    }
    let _ = delimiter;
    output
}

fn expand_replacement(value: &str, regex: &Regex, text: &str, range: Range<usize>) -> String {
    let captures = regex.captures(&text[range.clone()]);
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '&' {
            output.push_str(&text[range.clone()]);
            continue;
        }
        if character == '\\' && chars.peek() == Some(&'&') {
            chars.next();
            output.push('&');
            continue;
        }
        if character == '$' {
            let mut digits = String::new();
            while chars.peek().is_some_and(char::is_ascii_digit) {
                digits.push(chars.next().unwrap());
            }
            if !digits.is_empty() {
                if let Some(value) = captures
                    .as_ref()
                    .and_then(|c| c.get(digits.parse().unwrap()))
                {
                    output.push_str(value.as_str());
                }
                continue;
            }
        }
        output.push(character);
    }
    output
}

pub(crate) fn apply(plan: &SubstitutionPlan, accepted: &[usize]) -> String {
    let accepted = accepted
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut output = String::with_capacity(plan.source.len());
    let mut cursor = 0;
    for (index, item) in plan.matches.iter().enumerate() {
        if !accepted.contains(&index) {
            continue;
        }
        output.push_str(&plan.source[cursor..item.range.start]);
        output.push_str(&item.replacement);
        cursor = item.range.end;
    }
    output.push_str(&plan.source[cursor..]);
    output
}

fn line_offsets(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect()
}
