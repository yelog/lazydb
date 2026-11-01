mod catalog;
mod query;
mod relation;
mod transport;

pub use transport::SnowflakeAdapter;
pub(crate) use transport::invalid_response;

pub(crate) fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(crate) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}
