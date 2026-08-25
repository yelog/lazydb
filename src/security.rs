use std::fmt::Write;

use url::Url;

pub fn sanitize_terminal_text(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' | '\t' => sanitized.push(character),
            '\u{1b}' => sanitized.push_str("<ESC>"),
            '\r' => sanitized.push_str("<CR>"),
            value if value.is_control() => {
                let _ = write!(sanitized, "<0x{:02X}>", value as u32);
            }
            value => sanitized.push(value),
        }
    }
    sanitized
}

pub fn redact_connection_string(value: &str) -> String {
    let (prefix, raw) = if let Some(raw) = value.strip_prefix("jdbc:") {
        ("jdbc:", raw)
    } else {
        ("", value)
    };

    let Ok(mut url) = Url::parse(raw) else {
        return redact_fallback(value);
    };

    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }

    if url.query().is_some() {
        let pairs = url
            .query_pairs()
            .map(|(key, value)| {
                let replacement = if is_password_key(&key) {
                    "***".to_owned()
                } else {
                    value.into_owned()
                };
                (key.into_owned(), replacement)
            })
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }

    format!("{prefix}{url}")
}

fn is_password_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("password")
        || key.eq_ignore_ascii_case("passwd")
        || key.eq_ignore_ascii_case("pwd")
}

fn redact_fallback(value: &str) -> String {
    let mut output = value.to_owned();
    for key in ["password=", "passwd=", "pwd="] {
        let mut start = 0;
        while let Some(relative) = output[start..].to_ascii_lowercase().find(key) {
            let value_start = start + relative + key.len();
            let value_end = output[value_start..]
                .find(['&', ';'])
                .map(|offset| value_start + offset)
                .unwrap_or(output.len());
            output.replace_range(value_start..value_end, "***");
            start = value_start + 3;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{redact_connection_string, sanitize_terminal_text};

    #[test]
    fn removes_terminal_control_sequences() {
        let hostile = "safe\u{1b}]52;c;dGVzdA==\u{7} text\u{1b}[2J";
        let sanitized = sanitize_terminal_text(hostile);

        assert!(!sanitized.contains('\u{1b}'));
        assert!(!sanitized.contains('\u{7}'));
        assert!(sanitized.contains("<ESC>"));
        assert!(sanitized.contains("<0x07>"));
    }

    #[test]
    fn preserves_safe_newlines_and_tabs() {
        assert_eq!(sanitize_terminal_text("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn redacts_url_and_query_passwords() {
        let redacted = redact_connection_string(
            "jdbc:postgresql://alice:secret@db.example.com/app?password=also-secret&sslmode=require",
        );

        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("***"));
        assert!(redacted.starts_with("jdbc:"));
    }
}
