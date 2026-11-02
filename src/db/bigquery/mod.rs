mod auth;
mod catalog;
mod query;
mod relation;
mod transport;

pub(crate) use query::authorize_statement;
pub use transport::BigQueryAdapter;
pub(crate) use transport::invalid_response;

pub(crate) fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('\\', "\\\\").replace('`', "\\`"))
}

pub(crate) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}
