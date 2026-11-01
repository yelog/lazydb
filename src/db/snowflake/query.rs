use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;
use serde_json::Value;

use crate::db::{
    DatabaseError,
    query::{ColumnMeta, QueryBudget, QueryOutcome, QueryOutcomeAccumulator},
    value::CellValue,
};

use super::{SnowflakeAdapter, invalid_response};

#[derive(Clone, Debug, Deserialize)]
struct Column {
    name: String,
    #[serde(rename = "type")]
    native_type: String,
    scale: Option<i64>,
    precision: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    num_rows: u64,
    row_type: Vec<Column>,
    #[serde(default)]
    partition_info: Vec<Value>,
}

impl SnowflakeAdapter {
    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_with_budget(sql, QueryBudget::UNBOUNDED).await
    }

    pub(crate) async fn execute_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        if crate::sql::classify_statement_kind(sql, crate::sql::SqlDialect::Snowflake)
            == crate::sql::SqlStatementKind::Tcl
        {
            return Err(DatabaseError::unsupported(
                "Snowflake SQL API does not retain transactions across requests",
            ));
        }
        let mut accumulator = QueryOutcomeAccumulator::with_budget(budget);
        let mut response = self.submit(sql).await?;
        let metadata: Metadata = serde_json::from_value(
            response
                .get("resultSetMetaData")
                .cloned()
                .ok_or_else(|| invalid_response("Snowflake result metadata is missing"))?,
        )
        .map_err(|_| invalid_response("Snowflake result metadata is invalid"))?;
        let columns = metadata
            .row_type
            .iter()
            .map(|column| ColumnMeta {
                name: column.name.clone(),
                type_name: if column.native_type.eq_ignore_ascii_case("fixed") {
                    format!(
                        "NUMBER({},{})",
                        column.precision.unwrap_or(38),
                        column.scale.unwrap_or(0)
                    )
                } else {
                    column.native_type.to_ascii_uppercase()
                },
            })
            .collect::<Vec<_>>();
        accumulator.metadata(columns.clone());
        let handle = response
            .get("statementHandle")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let partitions = metadata.partition_info.len().max(1);
        let mut fetched = 0u64;
        let mut affected_rows = 0u64;
        for partition in 0..partitions {
            if partition > 0 {
                let remaining = accumulator.budget_remaining();
                if accumulator.budget_exhausted() || remaining.0 == 0 || remaining.1 == 0 {
                    if fetched < metadata.num_rows {
                        accumulator.mark_truncated();
                    }
                    break;
                }
                response = self
                    .partition(
                        handle.as_deref().ok_or_else(|| {
                            invalid_response("Snowflake partitioned result has no statement handle")
                        })?,
                        partition,
                    )
                    .await?;
            }
            let rows = response
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid_response("Snowflake result data is missing"))?;
            for row in rows {
                let row = row
                    .as_array()
                    .filter(|row| row.len() == metadata.row_type.len())
                    .ok_or_else(|| {
                        invalid_response("Snowflake result row does not match its columns")
                    })?;
                let values = row
                    .iter()
                    .zip(&metadata.row_type)
                    .map(|(value, column)| {
                        decode(value, column).map_err(|mut error| {
                            error.message = format!(
                                "{} (column {}, type {})",
                                error.message,
                                crate::security::sanitize_terminal_text(&column.name),
                                crate::security::sanitize_terminal_text(&column.native_type)
                            );
                            error
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for (value, column) in values.iter().zip(&metadata.row_type) {
                    if matches!(
                        column.name.to_ascii_lowercase().as_str(),
                        "number of rows inserted"
                            | "number of rows updated"
                            | "number of rows deleted"
                    ) {
                        affected_rows = affected_rows.saturating_add(match value {
                            CellValue::Integer(value) if *value >= 0 => *value as u64,
                            CellValue::Unsigned(value) => *value,
                            _ => 0,
                        });
                    }
                }
                fetched = fetched.saturating_add(1);
                accumulator.row(columns.clone(), values);
                if accumulator.budget_exhausted() {
                    break;
                }
            }
            if accumulator.budget_exhausted() {
                break;
            }
        }
        if !accumulator.budget_exhausted() && fetched != metadata.num_rows {
            return Err(invalid_response(
                "Snowflake partition row count does not match result metadata",
            ));
        }
        accumulator.done(affected_rows);
        Ok(accumulator.finish())
    }
}

fn decode(value: &Value, column: &Column) -> Result<CellValue, DatabaseError> {
    if value.is_null() {
        return Ok(CellValue::Null);
    }
    let text = value
        .as_str()
        .ok_or_else(|| invalid_response("Snowflake result cell must be a string or null"))?;
    let invalid = || invalid_response("Snowflake result cell does not match its declared type");
    Ok(match column.native_type.to_ascii_lowercase().as_str() {
        "fixed" if column.scale.unwrap_or(0) == 0 => text
            .parse::<i64>()
            .map(CellValue::Integer)
            .or_else(|_| text.parse::<u64>().map(CellValue::Unsigned))
            .unwrap_or_else(|_| CellValue::Text(text.to_owned())),
        // Snowflake NUMBER has up to 38 digits. Preserve decimals rather than round them to f64.
        "fixed" | "decfloat" => CellValue::Text(text.to_owned()),
        "real" | "float" => CellValue::Float(text.parse().map_err(|_| invalid())?),
        "boolean" => CellValue::Boolean(match text {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => return Err(invalid()),
        }),
        "binary" => {
            if text.len() % 2 != 0 || !text.is_ascii() {
                return Err(invalid());
            }
            let bytes = (0..text.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&text[index..index + 2], 16).map_err(|_| invalid()))
                .collect::<Result<Vec<_>, _>>()?;
            CellValue::Bytes(bytes)
        }
        "date" => CellValue::Date(
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .or_else(|| {
                    let days = text.parse::<i64>().ok()?;
                    NaiveDate::from_ymd_opt(1970, 1, 1)?
                        .checked_add_signed(chrono::Duration::try_days(days)?)
                })
                .ok_or_else(invalid)?,
        ),
        "time" => CellValue::Time(
            NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
                .ok()
                .or_else(|| {
                    let (seconds, nanos) = epoch_parts(text)?;
                    NaiveTime::from_num_seconds_from_midnight_opt(
                        u32::try_from(seconds).ok()?,
                        nanos,
                    )
                })
                .ok_or_else(invalid)?,
        ),
        "timestamp_ntz" => CellValue::DateTime(
            NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .or_else(|| {
                    let (seconds, nanos) = epoch_parts(text)?;
                    DateTime::from_timestamp(seconds, nanos).map(|value| value.naive_utc())
                })
                .ok_or_else(invalid)?,
        ),
        "timestamp_ltz" | "timestamp_tz" => CellValue::Timestamp(
            DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f %:z")
                .ok()
                .or_else(|| {
                    // SHOW metadata can ignore output parameters and return SQL API epoch seconds.
                    let (seconds, offset) = match text.split_once(' ') {
                        Some((seconds, offset)) => (seconds, offset.parse::<i32>().ok()?),
                        None => (text, 1440),
                    };
                    let offset =
                        chrono::FixedOffset::east_opt(offset.checked_sub(1440)?.checked_mul(60)?)?;
                    let (seconds, nanos) = epoch_parts(seconds)?;
                    Some(DateTime::from_timestamp(seconds, nanos)?.with_timezone(&offset))
                })
                .ok_or_else(invalid)?,
        ),
        // VARIANT/OBJECT/ARRAY and geography retain their complete textual representation.
        _ => CellValue::Text(text.to_owned()),
    })
}

// Parse decimal epoch seconds without f64 rounding, including times before 1970.
fn epoch_parts(text: &str) -> Option<(i64, u32)> {
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.bytes().all(|value| value.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|value| value.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?;
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u32>()
            .ok()?
            .checked_mul(10u32.pow(9 - fraction.len() as u32))?
    };
    if negative && nanos > 0 {
        Some((whole.checked_neg()?.checked_sub(1)?, 1_000_000_000 - nanos))
    } else {
        Some((
            if negative {
                whole.checked_neg()?
            } else {
                whole
            },
            nanos,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn column(kind: &str, scale: i64) -> Column {
        Column {
            name: "C".into(),
            native_type: kind.into(),
            scale: Some(scale),
            precision: Some(38),
        }
    }

    #[test]
    fn preserves_null_text_decimal_and_large_integer_without_precision_loss() {
        assert_eq!(
            decode(&Value::Null, &column("text", 0)).unwrap(),
            CellValue::Null
        );
        assert_eq!(
            decode(&json!("null"), &column("text", 0)).unwrap(),
            CellValue::Text("null".into())
        );
        for (text, scale) in [
            ("12345678901234567890123456789012345678", 0),
            ("12345678901234567890.123456789012345678", 18),
        ] {
            assert_eq!(
                decode(&json!(text), &column("fixed", scale)).unwrap(),
                CellValue::Text(text.into())
            );
        }
    }

    #[test]
    fn decodes_native_epoch_metadata_without_losing_fractional_precision() {
        let CellValue::Timestamp(value) =
            decode(&json!("1793497676.358"), &column("timestamp_ltz", 3)).unwrap()
        else {
            panic!("expected timestamp")
        };
        assert_eq!(value.timestamp(), 1793497676);
        assert_eq!(value.timestamp_subsec_nanos(), 358_000_000);
        let CellValue::Timestamp(value) =
            decode(&json!("-0.123456789 1770"), &column("timestamp_tz", 9)).unwrap()
        else {
            panic!("expected timestamp")
        };
        assert_eq!(value.timestamp(), -1);
        assert_eq!(value.timestamp_subsec_nanos(), 876_543_211);
        assert_eq!(value.offset().local_minus_utc(), 19800);
        assert!(matches!(
            decode(&json!("0"), &column("date", 0)).unwrap(),
            CellValue::Date(_)
        ));
        assert!(matches!(
            decode(&json!("45296.123456789"), &column("time", 9)).unwrap(),
            CellValue::Time(_)
        ));
        assert!(decode(&json!("1.1234567890"), &column("timestamp_ltz", 9)).is_err());
    }

    #[test]
    fn decodes_binary_and_temporal_values_and_rejects_malformed_cells() {
        assert_eq!(
            decode(&json!("00FF"), &column("binary", 0)).unwrap(),
            CellValue::Bytes(vec![0, 255])
        );
        assert!(matches!(
            decode(&json!("2026-10-31"), &column("date", 0)).unwrap(),
            CellValue::Date(_)
        ));
        assert!(matches!(
            decode(
                &json!("2026-10-31 12:34:56.123456789 +05:30"),
                &column("timestamp_tz", 0)
            )
            .unwrap(),
            CellValue::Timestamp(_)
        ));
        assert!(decode(&json!("bad"), &column("boolean", 0)).is_err());
        assert!(decode(&json!(true), &column("boolean", 0)).is_err());
        assert!(decode(&json!("☃a"), &column("binary", 0)).is_err());
    }
}
