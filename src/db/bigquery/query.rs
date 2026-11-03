use std::{collections::HashSet, sync::Arc, time::Duration};

use base64::Engine;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    BigQueryAdapter, invalid_response,
    transport::{
        JobGuard, JobIdentity, MAX_FETCH_BYTES, api_error, cancelled, retryable, safe_message,
    },
};
use crate::db::{
    DatabaseError, ErrorCategory,
    query::{BigQueryJobStats, ColumnMeta, QueryBudget, QueryOutcome, QueryOutcomeAccumulator},
    value::CellValue,
};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub fields: Vec<Field>,
}

impl BigQueryAdapter {
    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_with_budget(sql, QueryBudget::UNBOUNDED).await
    }

    pub(crate) async fn execute_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        self.execute_cancellable_with_budget(sql, budget, CancellationToken::new())
            .await
    }

    pub async fn execute_cancellable_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
        cancel: CancellationToken,
    ) -> Result<QueryOutcome, DatabaseError> {
        authorize_statement(sql, self.read_only)?;
        let mut guard = JobGuard {
            adapter: Arc::new(self.clone()),
            job: JobIdentity {
                project: self.project.clone(),
                id: format!("lazydb_{}", Uuid::new_v4().simple()),
                location: self.location.clone(),
            },
            armed: true,
        };
        let deadline = Instant::now() + self.timeout;
        // The guard exists before POST: a lost response or a dropped future must retain
        // the original identity, including for writes. Never resubmit this SQL.
        let mut remote_finished = false;
        let mut remote_may_have_started = false;
        let mut result = tokio::select! {
            biased;
            _ = cancelled(&cancel) => Err(self.cancel_job(&guard.job, "cancelled by caller").await),
            _ = tokio::time::sleep_until(deadline) => Err(self.cancel_job(&guard.job, "query deadline exceeded").await),
            result = self.run_job(sql, budget, &mut guard.job, &mut remote_finished, &mut remote_may_have_started) => result,
        };
        if let Err(error) = &mut result
            && remote_may_have_started
            && !remote_finished
            && error.category != ErrorCategory::Cancelled
        {
            let cancellation = self.cancel_job(&guard.job, "waiting failed").await;
            error.code = Some("job_outcome_unknown".into());
            error.message.push_str("; ");
            error.message.push_str(&cancellation.message);
        }
        guard.armed = false;
        result
    }

    async fn run_job(
        &self,
        sql: &str,
        budget: QueryBudget,
        job: &mut JobIdentity,
        remote_finished: &mut bool,
        remote_may_have_started: &mut bool,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut accumulator = QueryOutcomeAccumulator::with_budget(QueryBudget {
            max_rows: budget.max_rows.min(1_000_000),
            max_bytes: budget.max_bytes.min(64 * 1024 * 1024),
        });
        *remote_may_have_started = true;
        let submission = self
            .request(
                Method::POST,
                &["projects", &job.project, "jobs"],
                &[],
                Some(&self.job_body(sql, &job.id)),
            )
            .await;
        let mut job_resource = match submission {
            Ok((value, _)) => value,
            Err(error)
                if retryable(&error)
                    || error.code.as_deref() == Some("409")
                    || error.code.as_deref() == Some("duplicate")
                    || error.code.as_deref() == Some("bigquery_response") =>
            {
                match self
                    .get(&["projects", &job.project, "jobs", &job.id], &job.params())
                    .await
                {
                    Ok(value) => value,
                    Err(_) => {
                        return Err(DatabaseError {
                            category: ErrorCategory::Network,
                            code: Some("job_outcome_unknown".into()),
                            message: format!(
                                "BigQuery submission outcome is unknown; SQL was not resubmitted. Inspect project {}, job {}, location {} before retrying",
                                job.project,
                                job.id,
                                job.location.as_deref().unwrap_or("unspecified")
                            ),
                            diagnostic: None,
                        });
                    }
                }
            }
            Err(error) => {
                // A definitive submission rejection cannot have started this job.
                *remote_may_have_started = false;
                return Err(error);
            }
        };
        job.accept_reference(&job_resource)?;
        let mut delay = Duration::from_millis(100);
        loop {
            if job_resource
                .pointer("/status/state")
                .and_then(Value::as_str)
                == Some("DONE")
            {
                *remote_finished = true;
            }
            check_job_error(&job_resource)?;
            match job_resource
                .pointer("/status/state")
                .and_then(Value::as_str)
            {
                Some("DONE") => break,
                Some("PENDING" | "RUNNING") => {}
                _ => {
                    return Err(invalid_response(
                        "BigQuery job status is missing or invalid",
                    ));
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(2));
            job_resource = self
                .get(&["projects", &job.project, "jobs", &job.id], &job.params())
                .await?;
            job.accept_reference(&job_resource)?;
        }
        let stats = native_stats(job, &job_resource);
        let mut fields: Option<Vec<Field>> = None;
        let mut page_token: Option<String> = None;
        let mut seen_tokens = HashSet::new();
        let mut fetched = 0u64;
        let mut received = 0usize;
        let mut total_rows = None;
        for page_index in 0..10_000 {
            let remaining = accumulator.budget_remaining();
            if page_index > 0
                && (accumulator.budget_exhausted() || remaining.0 == 0 || remaining.1 == 0)
            {
                accumulator.mark_truncated();
                break;
            }
            let mut params = job.params();
            params.push(("maxResults", remaining.0.clamp(1, 1000).to_string()));
            params.push(("timeoutMs", "0".into()));
            if let Some(token) = &page_token {
                params.push(("pageToken", token.clone()));
            }
            let (value, size) = self
                .get_with_size(&["projects", &job.project, "queries", &job.id], &params)
                .await?;
            received = received.saturating_add(size);
            if received > MAX_FETCH_BYTES {
                return Err(invalid_response(
                    "BigQuery cumulative response budget exceeded",
                ));
            }
            job.accept_reference(&value)?;
            if value.get("jobComplete").and_then(Value::as_bool) != Some(true) {
                if page_index == 9999 {
                    return Err(invalid_response(
                        "BigQuery result completion polling limit exceeded",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            if fields.is_none() {
                fields = Some(match value.pointer("/schema/fields") {
                    Some(fields) => serde_json::from_value(fields.clone())
                        .map_err(|_| invalid_response("BigQuery result schema is invalid"))?,
                    None if value.get("rows").is_none() => vec![],
                    None => return Err(invalid_response("BigQuery result schema is missing")),
                });
                total_rows = optional_u64(&value, "totalRows")?;
            }
            let fields = fields.as_ref().expect("schema initialized");
            let columns = fields
                .iter()
                .map(|field| ColumnMeta {
                    name: field.name.clone(),
                    type_name: native_type(field),
                })
                .collect::<Vec<_>>();
            accumulator.metadata(columns.clone());
            if let Some(rows) = value.get("rows") {
                let rows = rows
                    .as_array()
                    .ok_or_else(|| invalid_response("BigQuery rows must be an array"))?;
                for row in rows {
                    let cells = row
                        .get("f")
                        .and_then(Value::as_array)
                        .filter(|cells| cells.len() == fields.len())
                        .ok_or_else(|| {
                            invalid_response("BigQuery result row does not match its schema")
                        })?;
                    let cells = cells
                        .iter()
                        .zip(fields)
                        .map(|(cell, field)| {
                            decode(
                                cell.get("v").ok_or_else(|| {
                                    invalid_response("BigQuery cell value is missing")
                                })?,
                                field,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    fetched = fetched.saturating_add(1);
                    accumulator.row(columns.clone(), cells);
                    if accumulator.budget_exhausted() {
                        break;
                    }
                }
            }
            page_token = value
                .get("pageToken")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(str::to_owned);
            if accumulator.budget_exhausted() {
                break;
            }
            if page_token.is_none() {
                if total_rows.is_some_and(|total| fetched != total) {
                    return Err(invalid_response(
                        "BigQuery result row count does not match totalRows",
                    ));
                }
                break;
            }
            if !seen_tokens.insert(page_token.clone().expect("nonempty page token")) {
                return Err(invalid_response(
                    "BigQuery result page token did not advance",
                ));
            }
            if page_index == 9999 {
                return Err(invalid_response(
                    "BigQuery result pagination limit exceeded",
                ));
            }
        }
        let affected = job_resource
            .pointer("/statistics/query/numDmlAffectedRows")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        accumulator.done(affected);
        let mut outcome = accumulator.finish();
        outcome.stats.bigquery = Some(Box::new(stats));
        Ok(outcome)
    }
}

pub(crate) fn authorize_statement(sql: &str, read_only: bool) -> Result<(), DatabaseError> {
    if crate::sql::scan_statements(sql, crate::sql::SqlDialect::BigQuery).len() != 1 {
        return Err(DatabaseError::unsupported(
            "BigQuery supports exactly one GoogleSQL statement per request",
        ));
    }
    use crate::sql::{SqlDialect, SqlRisk, SqlRiskAggregate, classify_sql};
    match classify_sql(sql, SqlDialect::BigQuery).aggregate {
        SqlRiskAggregate::Single(SqlRisk::ReadOnly) => Ok(()),
        SqlRiskAggregate::Single(SqlRisk::Dml | SqlRisk::Ddl) if read_only => Err(DatabaseError {
            category: ErrorCategory::Permission,
            code: Some("profile_read_only".into()),
            message: "This BigQuery connection is read-only; the statement was not sent".into(),
            diagnostic: None,
        }),
        SqlRiskAggregate::Single(SqlRisk::Dml | SqlRisk::Ddl) => Ok(()),
        _ => Err(DatabaseError::unsupported(
            "BigQuery scripts, unclassified statements, session controls and manual transactions are not supported",
        )),
    }
}

fn check_job_error(value: &Value) -> Result<(), DatabaseError> {
    if let Some(error) = value.pointer("/status/errorResult") {
        let reason = error
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("jobError");
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("job failed");
        let category = match reason {
            "accessDenied" => ErrorCategory::Permission,
            "authError" | "invalidCredentials" => ErrorCategory::Authentication,
            "stopped" => ErrorCategory::Cancelled,
            "backendError" | "internalError" | "rateLimitExceeded" => ErrorCategory::Network,
            _ => ErrorCategory::Sql,
        };
        let mut mapped = api_error(StatusCode::BAD_REQUEST, &Value::Null);
        mapped.category = category;
        let reason = super::transport::safe_reason(reason);
        mapped.code = Some(reason.clone());
        mapped.message = format!("BigQuery job {reason}: {}", safe_message(message));
        return Err(mapped);
    }
    Ok(())
}

fn native_stats(job: &JobIdentity, value: &Value) -> BigQueryJobStats {
    let query = value.pointer("/statistics/query").unwrap_or(&Value::Null);
    let number = |field| {
        query
            .get(field)
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
    };
    BigQueryJobStats {
        job_id: job.id.clone(),
        project: job.project.clone(),
        location: job.location.clone(),
        total_bytes_processed: number("totalBytesProcessed"),
        total_bytes_billed: number("totalBytesBilled"),
        slot_millis: number("totalSlotMs"),
        cache_hit: query.get("cacheHit").and_then(Value::as_bool),
    }
}

fn optional_u64(value: &Value, field: &str) -> Result<Option<u64>, DatabaseError> {
    value
        .get(field)
        .map(|value| {
            value
                .as_str()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| invalid_response("BigQuery returned invalid numeric metadata"))
        })
        .transpose()
}

pub(super) fn native_type(field: &Field) -> String {
    let kind = match field.kind.as_str() {
        "RECORD" | "STRUCT" => format!(
            "STRUCT<{}>",
            field
                .fields
                .iter()
                .map(|child| format!(
                    "{} {}",
                    super::quote_identifier(&child.name),
                    native_type(child)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => field.kind.clone(),
    };
    if field.mode == "REPEATED" {
        format!("ARRAY<{kind}>")
    } else {
        kind
    }
}

fn decode(value: &Value, field: &Field) -> Result<CellValue, DatabaseError> {
    if value.is_null() {
        return Ok(CellValue::Null);
    }
    if field.mode == "REPEATED" || matches!(field.kind.as_str(), "RECORD" | "STRUCT") {
        // Keep typed nested values as lossless JSON text in the existing scalar grid model.
        // Numeric strings remain strings so downstream JSON tools cannot round decimals.
        return Ok(CellValue::Text(nested(value, field)?.to_string()));
    }
    let text = value
        .as_str()
        .ok_or_else(|| invalid_response("BigQuery scalar cell must be a string or null"))?;
    let invalid = || invalid_response("BigQuery cell does not match its native type");
    Ok(match field.kind.as_str() {
        "BOOLEAN" | "BOOL" => CellValue::Boolean(match text {
            "true" => true,
            "false" => false,
            _ => return Err(invalid()),
        }),
        "INTEGER" | "INT64" => CellValue::Integer(text.parse().map_err(|_| invalid())?),
        "FLOAT" | "FLOAT64" => CellValue::Float(match text {
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            "NaN" => f64::NAN,
            _ => text.parse().map_err(|_| invalid())?,
        }),
        "NUMERIC" | "BIGNUMERIC" | "STRING" => CellValue::Text(text.into()),
        "BYTES" => CellValue::Bytes(
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .map_err(|_| invalid())?,
        ),
        "DATE" => {
            CellValue::Date(NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| invalid())?)
        }
        "TIME" => {
            CellValue::Time(NaiveTime::parse_from_str(text, "%H:%M:%S%.f").map_err(|_| invalid())?)
        }
        "DATETIME" => CellValue::DateTime(
            NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
                .map_err(|_| invalid())?,
        ),
        "TIMESTAMP" => {
            let (seconds, nanos) = epoch_parts(text).ok_or_else(invalid)?;
            CellValue::Timestamp(
                DateTime::from_timestamp(seconds, nanos)
                    .ok_or_else(invalid)?
                    .fixed_offset(),
            )
        }
        "JSON" => {
            serde_json::from_str::<Value>(text).map_err(|_| invalid())?;
            CellValue::Text(text.into())
        }
        _ => CellValue::Unsupported {
            type_name: native_type(field),
            preview: text.into(),
        },
    })
}

fn nested(value: &Value, field: &Field) -> Result<Value, DatabaseError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    if field.mode == "REPEATED" {
        let mut scalar = field.clone();
        scalar.mode.clear();
        return value
            .as_array()
            .ok_or_else(|| invalid_response("BigQuery repeated cell is invalid"))?
            .iter()
            .map(|cell| {
                nested(
                    cell.get("v")
                        .ok_or_else(|| invalid_response("BigQuery repeated value is missing"))?,
                    &scalar,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if matches!(field.kind.as_str(), "RECORD" | "STRUCT") {
        let values = value
            .get("f")
            .and_then(Value::as_array)
            .filter(|cells| cells.len() == field.fields.len())
            .ok_or_else(|| invalid_response("BigQuery STRUCT cell is invalid"))?;
        let members = values
            .iter()
            .zip(&field.fields)
            .map(|(cell, child)| {
                Ok((
                    child.name.clone(),
                    nested(
                        cell.get("v")
                            .ok_or_else(|| invalid_response("BigQuery STRUCT value is missing"))?,
                        child,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let mut names = HashSet::new();
        if members
            .iter()
            .all(|(name, _)| !name.is_empty() && names.insert(name.as_str()))
        {
            return Ok(Value::Object(members.into_iter().collect()));
        }
        // Anonymous or duplicate field names are legal STRUCTs. An object would
        // overwrite members, so retain names and ordinal order in this case.
        return Ok(Value::Array(
            members
                .into_iter()
                .map(|(name, value)| serde_json::json!({"name": name, "value": value}))
                .collect(),
        ));
    }
    decode(value, field)?;
    Ok(value.clone())
}

// REST TIMESTAMP values are decimal seconds since the Unix epoch, sometimes
// scientific notation. Parse with integer arithmetic, including negative epochs.
fn epoch_parts(text: &str) -> Option<(i64, u32)> {
    let (mantissa, exponent) = if let Some((base, exponent)) = text.split_once(['e', 'E']) {
        (base, exponent.parse::<i32>().ok()?)
    } else {
        (text, 0)
    };
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
        || whole.len() + fraction.len() > 30
    {
        return None;
    }
    let digits = format!("{whole}{fraction}").parse::<i128>().ok()?;
    let power = exponent
        .checked_sub(i32::try_from(fraction.len()).ok()?)?
        .checked_add(9)?;
    let nanos = if power >= 0 {
        digits.checked_mul(10i128.checked_pow(power.try_into().ok()?)?)?
    } else {
        let divisor = 10i128.checked_pow(power.checked_neg()?.try_into().ok()?)?;
        if digits % divisor != 0 {
            return None;
        }
        digits / divisor
    };
    let nanos = if negative {
        nanos.checked_neg()?
    } else {
        nanos
    };
    Some((
        i64::try_from(nanos.div_euclid(1_000_000_000)).ok()?,
        u32::try_from(nanos.rem_euclid(1_000_000_000)).ok()?,
    ))
}

#[cfg(test)]
mod tests;
