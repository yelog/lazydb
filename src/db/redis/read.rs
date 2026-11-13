use super::{RedisAdapter, types::RedisKeyId};
use crate::db::{DatabaseError, ErrorCategory};

pub const MAX_STRING_PREVIEW_BYTES: usize = 64 * 1024;
pub const MAX_COLLECTION_PREVIEW_ITEMS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedisType {
    String,
    Hash,
    List,
    Set,
    SortedSet,
    Stream,
    Module,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtlState {
    Missing,
    Persistent,
    ExpiresIn { millis: u64 },
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisKeyMetadata {
    pub key: RedisKeyId,
    pub value_type: RedisType,
    pub ttl: TtlState,
    /// Redis's allocator estimate for the complete key, not the loaded page.
    pub memory_usage_bytes: Option<u64>,
    /// String byte length or collection element count, according to type.
    pub value_size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RedisPagePosition {
    StringOffset(u64),
    HashCursor(u64),
    ListOffset(u64),
    SetCursor(u64),
    SortedSetOffset(u64),
    StreamId(Vec<u8>),
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RedisPageValue {
    String(Vec<u8>),
    Hash(Vec<(Vec<u8>, Vec<u8>)>),
    List(Vec<(u64, Vec<u8>)>),
    Set(Vec<Vec<u8>>),
    SortedSet(Vec<(Vec<u8>, Vec<u8>)>),
    Stream(Vec<RedisStreamEntry>),
}

pub type RedisStreamEntry = (Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisValuePage {
    pub metadata: RedisKeyMetadata,
    pub position: RedisPagePosition,
    pub value: RedisPageValue,
    pub truncated: bool,
    /// Whether this page represents the complete value under the current
    /// pagination contract. `truncated` instead describes a client budget.
    pub complete: bool,
    pub raw_bytes: usize,
    pub formatted_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RedisReadRequest {
    StringRange {
        key: RedisKeyId,
        start: u64,
        end: u64,
    },
    HashScan {
        key: RedisKeyId,
        cursor: u64,
        count: u32,
    },
    ListRange {
        key: RedisKeyId,
        start: u64,
        end: u64,
    },
    SetScan {
        key: RedisKeyId,
        cursor: u64,
        count: u32,
    },
    SortedSetRange {
        key: RedisKeyId,
        start: u64,
        end: u64,
    },
    StreamRange {
        key: RedisKeyId,
        start: Vec<u8>,
        count: u32,
    },
}

impl RedisReadRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::StringRange { start, end, .. } if end < start => {
                Err("string range end precedes start")
            }
            Self::StringRange { start, end, .. } => {
                match end.checked_sub(*start).and_then(|v| v.checked_add(1)) {
                    Some(length) if length <= MAX_STRING_PREVIEW_BYTES as u64 => Ok(()),
                    Some(_) => Err("string range exceeds preview budget"),
                    None => Err("string range exceeds preview budget"),
                }
            }
            Self::HashScan { count, .. } | Self::SetScan { count, .. }
                if *count as usize > MAX_COLLECTION_PREVIEW_ITEMS =>
            {
                Err("collection count exceeds preview budget")
            }
            Self::ListRange { start, end, .. } | Self::SortedSetRange { start, end, .. } => {
                match end.checked_sub(*start).and_then(|v| v.checked_add(1)) {
                    Some(length) if length <= MAX_COLLECTION_PREVIEW_ITEMS as u64 => Ok(()),
                    Some(_) | None => Err("collection range exceeds preview budget"),
                }
            }
            Self::StreamRange { count, .. } if *count as usize > MAX_COLLECTION_PREVIEW_ITEMS => {
                Err("stream count exceeds preview budget")
            }
            _ => Ok(()),
        }
    }
}

impl RedisType {
    fn parse(value: &str) -> Self {
        match value {
            "string" => Self::String,
            "hash" => Self::Hash,
            "list" => Self::List,
            "set" => Self::Set,
            "zset" => Self::SortedSet,
            "stream" => Self::Stream,
            "none" => Self::Missing,
            value if value.starts_with("module") => Self::Module,
            _ => Self::Unknown,
        }
    }
}

fn parse_ttl(value: i64) -> TtlState {
    match value {
        -2 => TtlState::Missing,
        -1 => TtlState::Persistent,
        value if value >= 0 => TtlState::ExpiresIn {
            millis: value as u64,
        },
        _ => TtlState::Unavailable,
    }
}

impl RedisAdapter {
    pub async fn key_metadata(&self, key: &RedisKeyId) -> Result<RedisKeyMetadata, DatabaseError> {
        if key.target.database != self.database() {
            return Err(DatabaseError::configuration(
                "Redis key target database mismatch",
            ));
        }
        let cache_key = self.metadata_cache_key(&key.key);
        if let Some(metadata) = self.metadata_cache_get(&cache_key) {
            return Ok(metadata);
        }
        let mut connection = self.connection_clone();
        let value_type: String = redis::cmd("TYPE")
            .arg(&key.key)
            .query_async(&mut connection)
            .await
            .map_err(|error| redis_error(error, ErrorCategory::Network))?;
        let ttl: i64 = redis::cmd("PTTL")
            .arg(&key.key)
            .query_async(&mut connection)
            .await
            .map_err(|error| redis_error(error, ErrorCategory::Network))?;
        let value_type = RedisType::parse(&value_type);
        let memory_usage_bytes = best_effort_u64(&mut connection, "MEMORY", |command| {
            command.arg("USAGE").arg(&key.key);
        })
        .await?;
        let value_size = match value_type {
            RedisType::String => {
                best_effort_u64(&mut connection, "STRLEN", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::Hash => {
                best_effort_u64(&mut connection, "HLEN", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::List => {
                best_effort_u64(&mut connection, "LLEN", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::Set => {
                best_effort_u64(&mut connection, "SCARD", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::SortedSet => {
                best_effort_u64(&mut connection, "ZCARD", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::Stream => {
                best_effort_u64(&mut connection, "XLEN", |command| {
                    command.arg(&key.key);
                })
                .await?
            }
            RedisType::Module | RedisType::Missing | RedisType::Unknown => None,
        };
        let metadata = RedisKeyMetadata {
            key: key.clone(),
            value_type,
            ttl: parse_ttl(ttl),
            memory_usage_bytes,
            value_size,
        };
        self.metadata_cache_insert(cache_key, metadata.clone());
        Ok(metadata)
    }

    pub async fn read_value_page(
        &self,
        request: &RedisReadRequest,
    ) -> Result<RedisValuePage, DatabaseError> {
        request.validate().map_err(DatabaseError::configuration)?;
        let key = match request {
            RedisReadRequest::StringRange { key, .. }
            | RedisReadRequest::HashScan { key, .. }
            | RedisReadRequest::ListRange { key, .. }
            | RedisReadRequest::SetScan { key, .. }
            | RedisReadRequest::SortedSetRange { key, .. }
            | RedisReadRequest::StreamRange { key, .. } => key,
        };
        let metadata = self.key_metadata(key).await?;
        self.read_value_page_with_metadata(request, metadata).await
    }

    pub async fn read_value_page_with_metadata(
        &self,
        request: &RedisReadRequest,
        metadata: RedisKeyMetadata,
    ) -> Result<RedisValuePage, DatabaseError> {
        request.validate().map_err(DatabaseError::configuration)?;
        let key = match request {
            RedisReadRequest::StringRange { key, .. }
            | RedisReadRequest::HashScan { key, .. }
            | RedisReadRequest::ListRange { key, .. }
            | RedisReadRequest::SetScan { key, .. }
            | RedisReadRequest::SortedSetRange { key, .. }
            | RedisReadRequest::StreamRange { key, .. } => key,
        };
        if metadata.value_type == RedisType::Missing {
            return Err(DatabaseError::configuration("Redis key is missing"));
        }
        if metadata.key != *key {
            return Err(DatabaseError::configuration(
                "Redis metadata key does not match the requested key",
            ));
        }
        let mut connection = self.connection_clone();
        let (value, position) = match request {
            RedisReadRequest::StringRange { start, end, .. } => {
                let value: Vec<u8> = redis::cmd("GETRANGE")
                    .arg(&key.key)
                    .arg(*start)
                    .arg(*end)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                let complete = metadata
                    .value_size
                    .is_some_and(|size| *end >= size.saturating_sub(1))
                    || value.len() < end.saturating_sub(*start).saturating_add(1) as usize;
                (
                    RedisPageValue::String(value),
                    if complete {
                        RedisPagePosition::Complete
                    } else {
                        RedisPagePosition::StringOffset(end.saturating_add(1))
                    },
                )
            }
            RedisReadRequest::HashScan { cursor, count, .. } => {
                let (next, values): (u64, Vec<Vec<u8>>) = redis::cmd("HSCAN")
                    .arg(&key.key)
                    .arg(*cursor)
                    .arg("COUNT")
                    .arg(*count)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                let pairs = values
                    .chunks(2)
                    .filter_map(|pair| {
                        pair.first()
                            .map(|field| (field.clone(), pair.get(1).cloned().unwrap_or_default()))
                    })
                    .collect();
                (
                    RedisPageValue::Hash(pairs),
                    if next == 0 {
                        RedisPagePosition::Complete
                    } else {
                        RedisPagePosition::HashCursor(next)
                    },
                )
            }
            RedisReadRequest::SetScan { cursor, count, .. } => {
                let (next, values): (u64, Vec<Vec<u8>>) = redis::cmd("SSCAN")
                    .arg(&key.key)
                    .arg(*cursor)
                    .arg("COUNT")
                    .arg(*count)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                (
                    RedisPageValue::Set(values),
                    if next == 0 {
                        RedisPagePosition::Complete
                    } else {
                        RedisPagePosition::SetCursor(next)
                    },
                )
            }
            RedisReadRequest::ListRange { start, end, .. } => {
                let values: Vec<Vec<u8>> = redis::cmd("LRANGE")
                    .arg(&key.key)
                    .arg(*start)
                    .arg(*end)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                let complete = metadata
                    .value_size
                    .is_some_and(|size| *end >= size.saturating_sub(1))
                    || values.len() < end.saturating_sub(*start).saturating_add(1) as usize;
                (
                    RedisPageValue::List(
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (*start + index as u64, value))
                            .collect(),
                    ),
                    if complete {
                        RedisPagePosition::Complete
                    } else {
                        RedisPagePosition::ListOffset(end.saturating_add(1))
                    },
                )
            }
            RedisReadRequest::SortedSetRange { start, end, .. } => {
                let values: Vec<Vec<u8>> = redis::cmd("ZRANGE")
                    .arg(&key.key)
                    .arg(*start)
                    .arg(*end)
                    .arg("WITHSCORES")
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                let complete = metadata
                    .value_size
                    .is_some_and(|size| *end >= size.saturating_sub(1))
                    || values.len().saturating_add(1) / 2
                        < end.saturating_sub(*start).saturating_add(1) as usize;
                let pairs = values
                    .chunks(2)
                    .filter_map(|pair| {
                        pair.first().map(|member| {
                            (member.clone(), pair.get(1).cloned().unwrap_or_default())
                        })
                    })
                    .collect();
                (
                    RedisPageValue::SortedSet(pairs),
                    if complete {
                        RedisPagePosition::Complete
                    } else {
                        RedisPagePosition::SortedSetOffset(end.saturating_add(1))
                    },
                )
            }
            RedisReadRequest::StreamRange { start, count, .. } => {
                let values: redis::Value = redis::cmd("XRANGE")
                    .arg(&key.key)
                    .arg(start)
                    .arg("+")
                    .arg("COUNT")
                    .arg(*count)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                let entries = parse_stream_entries(values)?;
                let position = entries
                    .last()
                    .map(|(id, _)| RedisPagePosition::StreamId(id.clone()))
                    .unwrap_or_else(|| RedisPagePosition::Complete);
                (RedisPageValue::Stream(entries), position)
            }
        };
        let raw_bytes = value_bytes(&value);
        let complete = matches!(position, RedisPagePosition::Complete);
        Ok(RedisValuePage {
            metadata,
            position,
            value,
            truncated: false,
            complete,
            raw_bytes,
            formatted_bytes: raw_bytes,
        })
    }

    pub async fn preview_key(&self, key: &RedisKeyId) -> Result<String, DatabaseError> {
        if key.target.database != self.database() {
            return Err(DatabaseError::configuration(
                "Redis key target database mismatch",
            ));
        }
        let mut connection = self.connection_clone();
        let value_type: String = redis::cmd("TYPE")
            .arg(&key.key)
            .query_async(&mut connection)
            .await
            .map_err(|error| redis_error(error, ErrorCategory::Network))?;
        let ttl: i64 = redis::cmd("PTTL")
            .arg(&key.key)
            .query_async(&mut connection)
            .await
            .map_err(|error| redis_error(error, ErrorCategory::Network))?;
        let header = format!("type={value_type} ttl={ttl}ms\n");
        let body = match value_type.as_str() {
            "none" => "<missing>".to_owned(),
            "string" => {
                let value: Vec<u8> = redis::cmd("GETRANGE")
                    .arg(&key.key)
                    .arg(0)
                    .arg(MAX_STRING_PREVIEW_BYTES as i64 - 1)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                display_bytes(&value)
            }
            "hash" => scan_pairs(&mut connection, "HSCAN", &key.key).await?,
            "set" => scan_values(&mut connection, "SSCAN", &key.key).await?,
            "list" => {
                let values: Vec<Vec<u8>> = redis::cmd("LRANGE")
                    .arg(&key.key)
                    .arg(0)
                    .arg(MAX_COLLECTION_PREVIEW_ITEMS as i64 - 1)
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| format!("{i}\t{}", display_bytes(value)))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "zset" => {
                let values: Vec<Vec<u8>> = redis::cmd("ZRANGE")
                    .arg(&key.key)
                    .arg(0)
                    .arg(MAX_COLLECTION_PREVIEW_ITEMS as i64 - 1)
                    .arg("WITHSCORES")
                    .query_async(&mut connection)
                    .await
                    .map_err(|error| redis_error(error, ErrorCategory::Network))?;
                values
                    .chunks(2)
                    .map(|pair| {
                        format!(
                            "{}\t{}",
                            display_bytes(&pair[0]),
                            pair.get(1).map_or(String::new(), |v| display_bytes(v))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            other => format!("<preview for {other} is not supported>"),
        };
        Ok(format!("{header}{body}"))
    }
}

fn value_bytes(value: &RedisPageValue) -> usize {
    match value {
        RedisPageValue::String(value) => value.len(),
        RedisPageValue::Hash(values) | RedisPageValue::SortedSet(values) => values
            .iter()
            .map(|(left, right)| left.len() + right.len())
            .sum(),
        RedisPageValue::List(values) => values.iter().map(|(_, value)| value.len()).sum(),
        RedisPageValue::Set(values) => values.iter().map(Vec::len).sum(),
        RedisPageValue::Stream(values) => values
            .iter()
            .map(|(id, fields)| {
                id.len() + fields.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
            })
            .sum(),
    }
}

pub fn parse_stream_entries(value: redis::Value) -> Result<Vec<RedisStreamEntry>, DatabaseError> {
    let redis::Value::Array(entries) = value else {
        return Err(DatabaseError::configuration(
            "invalid Redis stream response",
        ));
    };
    entries
        .into_iter()
        .map(|entry| {
            let redis::Value::Array(mut parts) = entry else {
                return Err(DatabaseError::configuration("invalid Redis stream entry"));
            };
            if parts.len() != 2 {
                return Err(DatabaseError::configuration(
                    "invalid Redis stream entry shape",
                ));
            }
            let id = stream_bytes_value(parts.remove(0))?;
            let redis::Value::Array(fields) = parts.remove(0) else {
                return Err(DatabaseError::configuration("invalid Redis stream fields"));
            };
            let mut pairs = Vec::new();
            for pair in fields.chunks(2) {
                pairs.push((stream_bytes(pair.first())?, stream_bytes(pair.get(1))?));
            }
            Ok((id, pairs))
        })
        .collect()
}

fn stream_bytes_value(value: redis::Value) -> Result<Vec<u8>, DatabaseError> {
    match value {
        redis::Value::BulkString(value) => Ok(value),
        redis::Value::SimpleString(value) => Ok(value.into_bytes()),
        _ => Err(DatabaseError::configuration("invalid Redis stream value")),
    }
}

fn stream_bytes(value: Option<&redis::Value>) -> Result<Vec<u8>, DatabaseError> {
    stream_bytes_value(
        value
            .cloned()
            .ok_or_else(|| DatabaseError::configuration("missing Redis stream field"))?,
    )
}

async fn best_effort_u64(
    connection: &mut super::connection::RedisConnection,
    command_name: &str,
    configure: impl FnOnce(&mut redis::Cmd),
) -> Result<Option<u64>, DatabaseError> {
    let mut command = redis::cmd(command_name);
    configure(&mut command);
    match command.query_async::<Option<u64>>(connection).await {
        Ok(value) => Ok(value),
        Err(error) if error.code().is_some() => Ok(None),
        Err(error) => Err(redis_error(error, ErrorCategory::Network)),
    }
}

async fn scan_values(
    connection: &mut super::connection::RedisConnection,
    command_name: &str,
    key: &[u8],
) -> Result<String, DatabaseError> {
    let reply: (u64, Vec<Vec<u8>>) = redis::cmd(command_name)
        .arg(key)
        .arg(0)
        .arg("COUNT")
        .arg(MAX_COLLECTION_PREVIEW_ITEMS)
        .query_async(connection)
        .await
        .map_err(|error| redis_error(error, ErrorCategory::Network))?;
    Ok(reply
        .1
        .iter()
        .map(|value| display_bytes(value))
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn scan_pairs(
    connection: &mut super::connection::RedisConnection,
    command_name: &str,
    key: &[u8],
) -> Result<String, DatabaseError> {
    let reply: (u64, Vec<Vec<u8>>) = redis::cmd(command_name)
        .arg(key)
        .arg(0)
        .arg("COUNT")
        .arg(MAX_COLLECTION_PREVIEW_ITEMS)
        .query_async(connection)
        .await
        .map_err(|error| redis_error(error, ErrorCategory::Network))?;
    Ok(reply
        .1
        .chunks(2)
        .map(|pair| {
            format!(
                "{}\t{}",
                display_bytes(&pair[0]),
                pair.get(1).map_or(String::new(), |v| display_bytes(v))
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn display_bytes(value: &[u8]) -> String {
    String::from_utf8(value.to_vec())
        .unwrap_or_else(|_| value.iter().map(|byte| format!("\\x{byte:02x}")).collect())
}

fn redis_error(error: redis::RedisError, category: ErrorCategory) -> DatabaseError {
    DatabaseError {
        category,
        code: error.code().map(str::to_owned),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_redis_types_and_ttl_states() {
        assert_eq!(RedisType::parse("string"), RedisType::String);
        assert_eq!(RedisType::parse("module.foo"), RedisType::Module);
        assert_eq!(RedisType::parse("future"), RedisType::Unknown);
        assert_eq!(parse_ttl(-2), TtlState::Missing);
        assert_eq!(parse_ttl(-1), TtlState::Persistent);
        assert_eq!(parse_ttl(42), TtlState::ExpiresIn { millis: 42 });
        assert_eq!(parse_ttl(-3), TtlState::Unavailable);
    }

    #[test]
    fn value_page_byte_accounting_keeps_binary_lengths() {
        let value = RedisPageValue::Hash(vec![(vec![0, 1], vec![2, 3, 4])]);
        assert_eq!(value_bytes(&value), 5);
    }
}
