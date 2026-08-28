pub(crate) struct DdlSection<'a> {
    pub label: &'a str,
    pub statements: Vec<String>,
}

pub(crate) fn assemble_ddl(sections: Vec<DdlSection<'_>>) -> Option<String> {
    let mut parts = Vec::new();

    for section in sections {
        let statements = section
            .statements
            .iter()
            .map(|statement| statement.trim())
            .filter(|statement| !statement.is_empty())
            .map(|statement| {
                if statement.ends_with(';') {
                    statement.to_owned()
                } else {
                    format!("{statement};")
                }
            })
            .collect::<Vec<_>>();

        if !statements.is_empty() {
            parts.push(format!(
                "-- {}\n\n{}",
                section.label,
                statements.join("\n\n")
            ));
        }
    }

    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::{DdlSection, assemble_ddl};

    #[test]
    fn returns_none_when_every_statement_is_empty() {
        assert_eq!(assemble_ddl(vec![]), None);
        assert_eq!(
            assemble_ddl(vec![
                DdlSection {
                    label: "Table",
                    statements: vec!["".to_owned(), "  \n\t".to_owned()],
                },
                DdlSection {
                    label: "Indexes",
                    statements: vec![],
                },
            ]),
            None
        );
    }

    #[test]
    fn formats_non_empty_sections_and_normalizes_statements() {
        let ddl = assemble_ddl(vec![
            DdlSection {
                label: "Table",
                statements: vec!["  CREATE TABLE users (id INTEGER)  ".to_owned()],
            },
            DdlSection {
                label: "Empty",
                statements: vec!["  ".to_owned()],
            },
            DdlSection {
                label: "Indexes",
                statements: vec![
                    "CREATE INDEX users_id_idx ON users (id);".to_owned(),
                    "\nCREATE UNIQUE INDEX users_name_idx ON users (name)\n".to_owned(),
                ],
            },
        ]);

        assert_eq!(
            ddl.as_deref(),
            Some(
                "-- Table\n\nCREATE TABLE users (id INTEGER);\n\n-- Indexes\n\nCREATE INDEX users_id_idx ON users (id);\n\nCREATE UNIQUE INDEX users_name_idx ON users (name);"
            )
        );
    }

    #[test]
    fn preserves_multiline_trigger_body_ending_in_semicolon() {
        let trigger = "CREATE TRIGGER users_audit\nAFTER UPDATE ON users\nBEGIN\n  INSERT INTO audit_log (user_id) VALUES (NEW.id);\n  UPDATE counters SET value = value + 1;\nEND;";

        let ddl = assemble_ddl(vec![DdlSection {
            label: "Triggers",
            statements: vec![trigger.to_owned()],
        }]);

        assert_eq!(ddl, Some(format!("-- Triggers\n\n{trigger}")));
    }
}
