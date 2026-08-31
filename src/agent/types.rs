use serde::Serialize;

use crate::{
    db::{query::QueryOutcome, value::CellValue},
    profile::{DatabaseKind, Environment},
};

pub const AGENT_API_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct AgentResponse<T: Serialize> {
    pub api_version: u16,
    pub ok: bool,
    pub result: T,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentErrorResponse {
    pub api_version: u16,
    pub ok: bool,
    pub error: AgentErrorBody,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentConnection {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub kind: DatabaseKind,
    pub environment: Environment,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub user: Option<String>,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentTarget {
    pub connection: AgentConnection,
    pub schema: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentQueryResult {
    pub target: AgentTarget,
    pub outcome: QueryOutcomeJson,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryOutcomeJson {
    pub result_sets: Vec<ResultSetJson>,
    pub execution_ms: u128,
    pub fetch_ms: u128,
    pub row_count: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResultSetJson {
    pub columns: Vec<crate::db::query::ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
}

impl From<QueryOutcome> for QueryOutcomeJson {
    fn from(outcome: QueryOutcome) -> Self {
        Self {
            result_sets: outcome
                .result_sets
                .into_iter()
                .map(|result| ResultSetJson {
                    columns: result.columns,
                    rows: result.rows,
                    affected_rows: result.affected_rows,
                })
                .collect(),
            execution_ms: outcome.stats.execution.as_millis(),
            fetch_ms: outcome.stats.fetch.as_millis(),
            row_count: outcome.stats.row_count,
            truncated: false,
        }
    }
}
