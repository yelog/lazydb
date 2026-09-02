use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlServerBatchError {
    count: String,
}

impl SqlServerBatchError {
    pub fn count(&self) -> &str {
        &self.count
    }
}

impl fmt::Display for SqlServerBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SQL Server GO repeat count {} is unsupported; only GO and GO 1 are allowed",
            self.count
        )
    }
}

impl std::error::Error for SqlServerBatchError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Normal,
    SingleQuote,
    DoubleQuote,
    Bracket,
    LineComment,
    BlockComment,
}

/// Splits SQL Server input on standalone `GO` lines without sending the
/// client-side separator to SQL Server.
pub fn split_sql_server_batches(sql: &str) -> Result<Vec<&str>, SqlServerBatchError> {
    let bytes = sql.as_bytes();
    let mut batches = Vec::new();
    let mut batch_start = 0;
    let mut line_start = 0;
    let mut state = State::Normal;
    let mut block_depth = 0usize;

    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |offset| line_start + offset);
        let content_end = if line_end > line_start && bytes[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let mut code = String::new();
        let mut index = line_start;

        while index < content_end {
            match state {
                State::Normal => match bytes[index] {
                    b'\'' => {
                        code.push('\'');
                        state = State::SingleQuote;
                        index += 1;
                    }
                    b'"' => {
                        code.push('"');
                        state = State::DoubleQuote;
                        index += 1;
                    }
                    b'[' => {
                        code.push('[');
                        state = State::Bracket;
                        index += 1;
                    }
                    b'-' if bytes.get(index + 1) == Some(&b'-') => {
                        state = State::LineComment;
                        index = content_end;
                    }
                    b'/' if bytes.get(index + 1) == Some(&b'*') => {
                        block_depth = 1;
                        state = State::BlockComment;
                        index += 2;
                    }
                    byte => {
                        code.push(byte as char);
                        index += 1;
                    }
                },
                State::SingleQuote => {
                    code.push('x');
                    if bytes[index] == b'\'' {
                        if bytes.get(index + 1) == Some(&b'\'') {
                            index += 2;
                        } else {
                            state = State::Normal;
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
                State::DoubleQuote => {
                    code.push('x');
                    if bytes[index] == b'"' {
                        if bytes.get(index + 1) == Some(&b'"') {
                            index += 2;
                        } else {
                            state = State::Normal;
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
                State::Bracket => {
                    code.push('x');
                    if bytes[index] == b']' {
                        if bytes.get(index + 1) == Some(&b']') {
                            index += 2;
                        } else {
                            state = State::Normal;
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
                State::LineComment => unreachable!("line comments reset at line boundaries"),
                State::BlockComment => {
                    if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                        block_depth += 1;
                        index += 2;
                    } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                        block_depth -= 1;
                        index += 2;
                        if block_depth == 0 {
                            state = State::Normal;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
        }

        if state == State::LineComment {
            state = State::Normal;
        }
        if let Some(count) = go_count(&code) {
            if count != "1" {
                return Err(SqlServerBatchError {
                    count: count.to_owned(),
                });
            }
            push_nonempty(&mut batches, &sql[batch_start..line_start]);
            batch_start = if line_end < bytes.len() {
                line_end + 1
            } else {
                line_end
            };
        }
        line_start = if line_end < bytes.len() {
            line_end + 1
        } else {
            line_end
        };
    }

    push_nonempty(&mut batches, &sql[batch_start..]);
    Ok(batches)
}

fn go_count(code: &str) -> Option<&str> {
    let mut tokens = code.split_whitespace();
    if !tokens.next()?.eq_ignore_ascii_case("GO") {
        return None;
    }
    let count = tokens.next().unwrap_or("1");
    if tokens.next().is_some() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(count)
}

fn push_nonempty<'a>(batches: &mut Vec<&'a str>, batch: &'a str) {
    let batch = batch.trim();
    if !batch.is_empty() {
        batches.push(batch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_crlf_and_allows_comments_and_go_one() {
        let sql = "SELECT 1\r\nGO -- next\r\n/* before */ GO 1 /* after */\r\nSELECT 2";
        assert_eq!(
            split_sql_server_batches(sql).unwrap(),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn ignores_go_inside_quoted_text_identifiers_and_nested_comments() {
        let sql = "SELECT 'GO', \"GO\", [GO]]name];\n/* outer\n/* GO */\nGO\n*/\nSELECT 2";
        assert_eq!(split_sql_server_batches(sql).unwrap(), vec![sql]);
    }

    #[test]
    fn rejects_repeat_counts_other_than_one() {
        let error = split_sql_server_batches("SELECT 1\nGO 2\nSELECT 2").unwrap_err();
        assert_eq!(error.count(), "2");
        assert!(error.to_string().contains("only GO and GO 1"));
    }
}
