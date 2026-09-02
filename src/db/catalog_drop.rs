use thiserror::Error;
use uuid::Uuid;

use crate::{
    db::catalog::{CatalogEntry, CatalogId, CatalogKind},
    identity::ConnectionIdentity,
};

/// The catalog identity used to build a drop plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDropRequest {
    pub connection: ConnectionIdentity,
    pub request_id: u64,
    pub catalog_epoch: u64,
    pub object: CatalogId,
    pub entry: Option<CatalogEntry>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogDropError {
    #[error("catalog drop request has an empty object ID")]
    EmptyObjectId,
    #[error(
        "catalog drop request profile {object_profile_id} does not match connection profile {connection_profile_id}"
    )]
    ProfileMismatch {
        object_profile_id: Uuid,
        connection_profile_id: Uuid,
    },
    #[error("catalog drop SQL is empty")]
    EmptySql,
    #[error("catalog drop SQL contains a forbidden clause")]
    UnsafeSql,
    #[error("catalog drop entry kind {entry_kind:?} does not match its ID kind {id_kind:?}")]
    KindMismatch {
        id_kind: CatalogKind,
        entry_kind: CatalogKind,
    },
    #[error("catalog drop entry ID does not match the request")]
    ObjectMismatch,
    #[error("catalog drop object name cannot be empty")]
    EmptyObjectName,
    #[error("catalog drop for {kind:?} is unsupported: {reason}")]
    Unsupported { kind: CatalogKind, reason: String },
}

impl CatalogDropRequest {
    pub fn new(connection: ConnectionIdentity, object: CatalogId, request_id: u64) -> Self {
        Self {
            connection,
            request_id,
            catalog_epoch: 0,
            object,
            entry: None,
        }
    }

    pub fn with_entry(mut self, entry: CatalogEntry) -> Self {
        self.entry = Some(entry);
        self
    }

    pub fn validate(&self) -> Result<(), CatalogDropError> {
        validate_object_id(&self.object, self.connection.profile_id)
    }
}

/// A validated, execution-ready description of one catalog object drop.
///
/// SQL generation and execution are deliberately outside this model. Consumers
/// must retain the request identity and revalidate it at the execution boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDropPlan {
    pub request: CatalogDropRequest,
    pub object: CatalogId,
    pub kind: CatalogKind,
    pub qualified_name: String,
    sql: String,
}

impl CatalogDropPlan {
    pub fn new(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
        sql: impl Into<String>,
    ) -> Result<Self, CatalogDropError> {
        request.validate()?;
        if entry.id != request.object {
            return Err(CatalogDropError::ObjectMismatch);
        }
        if entry.kind != entry.id.kind {
            return Err(CatalogDropError::KindMismatch {
                id_kind: entry.id.kind,
                entry_kind: entry.kind,
            });
        }
        if entry.qualified_name.object.is_empty() {
            return Err(CatalogDropError::EmptyObjectName);
        }
        let sql = sql.into();
        validate_sql(&sql)?;

        Ok(Self {
            request,
            object: entry.id.clone(),
            kind: entry.kind,
            qualified_name: entry.qualified_name.object.clone(),
            sql,
        })
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn validate(&self) -> Result<(), CatalogDropError> {
        self.request.validate()?;
        if self.object != self.request.object {
            return Err(CatalogDropError::ObjectMismatch);
        }
        if self.kind != self.object.kind {
            return Err(CatalogDropError::KindMismatch {
                id_kind: self.object.kind,
                entry_kind: self.kind,
            });
        }
        if self.qualified_name.is_empty() {
            return Err(CatalogDropError::EmptyObjectName);
        }
        validate_sql(&self.sql)
    }
}

fn validate_object_id(
    object: &CatalogId,
    connection_profile_id: Uuid,
) -> Result<(), CatalogDropError> {
    if object.native_path.is_empty() {
        return Err(CatalogDropError::EmptyObjectId);
    }
    if object.profile_id() != connection_profile_id {
        return Err(CatalogDropError::ProfileMismatch {
            object_profile_id: object.profile_id(),
            connection_profile_id,
        });
    }
    Ok(())
}

fn validate_sql(sql: &str) -> Result<(), CatalogDropError> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err(CatalogDropError::EmptySql);
    }
    let mut token = String::new();
    let mut tokens = Vec::new();
    let mut quoted = None;
    let mut bracketed = false;
    let mut characters = sql.chars().peekable();
    while let Some(character) = characters.next() {
        if bracketed {
            if character == ']' && characters.next_if_eq(&']').is_none() {
                bracketed = false;
            }
            continue;
        }
        if let Some(delimiter) = quoted {
            if character == delimiter {
                quoted = None;
            }
            continue;
        }
        if matches!(character, '"' | '`') {
            quoted = Some(character);
        } else if character == '[' {
            bracketed = true;
        } else if character.is_ascii_alphanumeric() || character == '_' {
            token.push(character.to_ascii_uppercase());
        } else {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            if character == ';' {
                return Err(CatalogDropError::UnsafeSql);
            }
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    if tokens.iter().any(|token| token == "CASCADE")
        || tokens.windows(2).any(|tokens| tokens == ["IF", "EXISTS"])
    {
        return Err(CatalogDropError::UnsafeSql);
    }
    Ok(())
}
