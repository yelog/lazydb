use super::super::query::ColumnMeta;
use super::super::value::CellValue;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::{
    Column, Row, TypeInfo, ValueRef,
    postgres::{
        PgRow,
        types::{PgInterval, PgPoint},
    },
    types::{BigDecimal, Json, JsonRawValue, ipnet::IpNet},
};
use uuid::Uuid;

pub(crate) fn columns(row: &PgRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

pub(crate) fn decode_row(row: &PgRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

pub(crate) fn decode_cell(row: &PgRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return unsupported("unknown", "decode error");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    let decoded = match type_name.as_str() {
        "BOOL" => row.try_get::<bool, _>(index).map(CellValue::Boolean),
        "INT2" => row
            .try_get::<i16, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT4" => row
            .try_get::<i32, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT8" => row.try_get::<i64, _>(index).map(CellValue::Integer),
        "FLOAT4" => row
            .try_get::<f32, _>(index)
            .map(|value| CellValue::Float(f64::from(value))),
        "FLOAT8" => row.try_get::<f64, _>(index).map(CellValue::Float),
        "DATE" => row.try_get::<NaiveDate, _>(index).map(CellValue::Date),
        "TIME" => row.try_get::<NaiveTime, _>(index).map(CellValue::Time),
        "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(index)
            .map(CellValue::DateTime),
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(index)
            .map(|value| CellValue::Timestamp(value.fixed_offset())),
        "NUMERIC" => row
            .try_get::<BigDecimal, _>(index)
            .map(|value| CellValue::Text(value.to_string())),
        "INTERVAL" => return decode_pg_interval(row, index),
        "UUID" => row
            .try_get::<Uuid, _>(index)
            .map(|value| CellValue::Text(value.to_string())),
        "JSON" | "JSONB" => row
            .try_get::<Json<Box<JsonRawValue>>, _>(index)
            .map(|value| CellValue::Text(value.0.get().to_owned())),
        "TEXT[]" | "VARCHAR[]" | "CHAR[]" | "NAME[]" => row
            .try_get::<Vec<Option<String>>, _>(index)
            .map(|value| CellValue::Text(format_pg_array(value))),
        "INET" | "CIDR" => return decode_pg_network(row, index, &type_name),
        "POINT" => row
            .try_get::<PgPoint, _>(index)
            .map(|value| CellValue::Text(format_pg_point(value))),
        "BYTEA" => row.try_get::<Vec<u8>, _>(index).map(CellValue::Bytes),
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => {
            row.try_get::<String, _>(index).map(CellValue::Text)
        }
        _ => return fallback_pg(row, index, &type_name),
    };
    decoded.unwrap_or_else(|error| unsupported(&type_name, &error.to_string()))
}

fn fallback_pg(row: &PgRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        CellValue::Text(value)
    } else if let Ok(value) = row.try_get_unchecked::<Vec<u8>, _>(index) {
        CellValue::Bytes(value)
    } else {
        unsupported(type_name, "unsupported PostgreSQL value")
    }
}

fn decode_pg_interval(row: &PgRow, index: usize) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<PgInterval, _>(index) {
        return CellValue::Text(format_pg_interval(value));
    }
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        return parse_pg_interval(&value)
            .map(format_pg_interval)
            .map(CellValue::Text)
            .unwrap_or(CellValue::Text(value));
    }
    unsupported("INTERVAL", "unsupported PostgreSQL interval")
}

pub(crate) fn parse_pg_interval(value: &str) -> Option<PgInterval> {
    let mut months = 0_i32;
    let mut days = 0_i32;
    let mut microseconds = 0_i64;
    let mut parsed = false;
    let tokens = value
        .trim()
        .trim_start_matches('@')
        .split_whitespace()
        .collect::<Vec<_>>();
    let ago = tokens
        .last()
        .is_some_and(|token| token.eq_ignore_ascii_case("ago"));
    let mut index = 0;

    while index < tokens.len() {
        let token = tokens[index];
        if token.eq_ignore_ascii_case("ago") {
            index += 1;
            continue;
        }
        if token.contains(':') {
            microseconds = microseconds.checked_add(parse_pg_interval_time(token)?)?;
            parsed = true;
            index += 1;
            continue;
        }
        let unit = *tokens.get(index + 1)?;
        match unit.trim_end_matches('s').to_ascii_lowercase().as_str() {
            "sec" => microseconds = microseconds.checked_add(parse_pg_seconds(token)?)?,
            "year" | "mon" | "day" | "hour" | "min" => {
                let number = token.parse::<i64>().ok()?;
                match unit.trim_end_matches('s').to_ascii_lowercase().as_str() {
                    "year" => {
                        months = months.checked_add(i32::try_from(number.checked_mul(12)?).ok()?)?
                    }
                    "mon" => months = months.checked_add(i32::try_from(number).ok()?)?,
                    "day" => days = days.checked_add(i32::try_from(number).ok()?)?,
                    "hour" => {
                        microseconds =
                            microseconds.checked_add(number.checked_mul(3_600_000_000)?)?
                    }
                    "min" => {
                        microseconds = microseconds.checked_add(number.checked_mul(60_000_000)?)?
                    }
                    _ => unreachable!(),
                }
            }
            _ => return None,
        }
        parsed = true;
        index += 2;
    }

    parsed.then_some(PgInterval {
        months: if ago { months.checked_neg()? } else { months },
        days: if ago { days.checked_neg()? } else { days },
        microseconds: if ago {
            microseconds.checked_neg()?
        } else {
            microseconds
        },
    })
}

fn parse_pg_interval_time(value: &str) -> Option<i64> {
    let negative = value.starts_with('-');
    let value = value.trim_start_matches(['+', '-']);
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<i64>().ok()?;
    let minutes = parts.next()?.parse::<i64>().ok()?;
    let seconds = parse_pg_seconds(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    let total = hours
        .checked_mul(3_600_000_000)?
        .checked_add(minutes.checked_mul(60_000_000)?)?
        .checked_add(seconds)?;
    Some(if negative { -total } else { total })
}

fn parse_pg_seconds(value: &str) -> Option<i64> {
    let negative = value.starts_with('-');
    let value = value.trim_start_matches(['+', '-']);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 6 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?.checked_mul(1_000_000)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()? * 10_i64.pow(6 - fraction.len() as u32)
    };
    let value = whole.checked_add(fraction)?;
    Some(if negative {
        value.checked_neg()?
    } else {
        value
    })
}

pub(crate) fn format_pg_interval(value: PgInterval) -> String {
    const MICROS_PER_SECOND: i64 = 1_000_000;
    const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
    const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;

    let years = value.months / 12;
    let months = value.months % 12;
    let hours = value.microseconds / MICROS_PER_HOUR;
    let minutes = value.microseconds % MICROS_PER_HOUR / MICROS_PER_MINUTE;
    let seconds_micros = value.microseconds % MICROS_PER_MINUTE;
    let seconds = seconds_micros / MICROS_PER_SECOND;
    let fraction = seconds_micros.unsigned_abs() % MICROS_PER_SECOND as u64;
    let seconds = if fraction == 0 {
        format!("{seconds}.0")
    } else {
        let sign = if seconds == 0 && seconds_micros < 0 {
            "-"
        } else {
            ""
        };
        format!("{sign}{seconds}.{fraction:06}")
            .trim_end_matches('0')
            .to_owned()
    };

    format!(
        "{years} years {months} mons {} days {hours} hours {minutes} mins {seconds} secs",
        value.days
    )
}

pub(crate) fn format_pg_array(values: Vec<Option<String>>) -> String {
    let values = values
        .into_iter()
        .map(|value| match value {
            None => "NULL".to_owned(),
            Some(value) if pg_array_value_needs_quotes(&value) => {
                format!("\"{}\"", value.replace('\\', "\\\\").replace('\"', "\\\""))
            }
            Some(value) => value,
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{values}}}")
}

fn pg_array_value_needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value.eq_ignore_ascii_case("NULL")
        || value.chars().any(|character| {
            character.is_whitespace() || matches!(character, ',' | '{' | '}' | '"' | '\\')
        })
}

fn format_pg_network(value: IpNet) -> String {
    match value {
        IpNet::V4(value) if value.prefix_len() == 32 => value.addr().to_string(),
        IpNet::V6(value) if value.prefix_len() == 128 => value.addr().to_string(),
        value => value.to_string(),
    }
}

fn decode_pg_network(row: &PgRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        return CellValue::Text(value);
    }
    row.try_get_unchecked::<IpNet, _>(index)
        .map(|value| CellValue::Text(format_pg_network(value)))
        .unwrap_or_else(|error| unsupported(type_name, &error.to_string()))
}

fn format_pg_point(value: PgPoint) -> String {
    format!(
        "({},{})",
        format_pg_coordinate(value.x),
        format_pg_coordinate(value.y)
    )
}

fn format_pg_coordinate(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn unsupported(type_name: &str, preview: &str) -> CellValue {
    CellValue::Unsupported {
        type_name: type_name.to_owned(),
        preview: preview.to_owned(),
    }
}
