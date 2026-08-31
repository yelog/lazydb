use serde::Serialize;

use crate::{
    db::{
        DatabaseConnection,
        catalog::{CatalogSearchPage, CatalogSearchRequest, OptionalMetadata},
    },
    identity::ConnectionIdentity,
    profile::ConnectionProfile,
};

use super::{
    selection::{AgentError, AgentErrorCode},
    types::AgentTarget,
};

#[derive(Clone, Debug, Serialize)]
pub struct SchemaSearchResult {
    pub target: AgentTarget,
    pub hits: Vec<SchemaSearchHit>,
    pub total_count: Option<usize>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchemaSearchHit {
    pub kind: crate::db::catalog::CatalogKind,
    pub name: String,
    pub qualified_path: String,
    pub native_kind: String,
    pub comment: Option<String>,
}

pub async fn search_schema(
    connection: &DatabaseConnection,
    profile: &ConnectionProfile,
    target: AgentTarget,
    query: String,
    limit: usize,
) -> Result<SchemaSearchResult, AgentError> {
    let request = CatalogSearchRequest {
        connection: ConnectionIdentity {
            profile_id: profile.id,
            generation: 0,
        },
        session_id: 1,
        generation: 1,
        query,
        scope: profile.catalog_scope.clone(),
        limit,
    };
    let page: CatalogSearchPage =
        connection
            .search_catalog(&request)
            .await
            .map_err(|error| AgentError {
                code: AgentErrorCode::DatabaseFailure,
                message: error.to_string(),
            })?;
    Ok(SchemaSearchResult {
        target,
        hits: page
            .hits
            .into_iter()
            .map(|hit| SchemaSearchHit {
                kind: hit.entry.kind,
                name: hit.entry.qualified_name.object.clone(),
                qualified_path: hit.qualified_path(),
                native_kind: hit.entry.native_kind,
                comment: match hit.entry.comment {
                    OptionalMetadata::Supported(value) => value,
                    OptionalMetadata::Unsupported => None,
                },
            })
            .collect(),
        total_count: page.total_count,
        truncated: page.truncated,
    })
}
