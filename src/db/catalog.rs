use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    identity::ConnectionIdentity,
    profile::{CatalogScope, CatalogScopeValidationError},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Database,
    Schema,
    Table,
    View,
    MaterializedView,
    Column,
    Index,
    PrimaryKey,
    UniqueConstraint,
    ForeignKey,
    CheckConstraint,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Type,
}

impl CatalogKind {
    pub const fn is_relation(self) -> bool {
        matches!(self, Self::Table | Self::View | Self::MaterializedView)
    }

    pub const fn is_relation_child(self) -> bool {
        matches!(
            self,
            Self::Column
                | Self::Index
                | Self::PrimaryKey
                | Self::UniqueConstraint
                | Self::ForeignKey
                | Self::CheckConstraint
                | Self::Trigger
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CatalogId {
    pub connection_id: Uuid,
    pub kind: CatalogKind,
    pub native_path: Vec<String>,
}

impl CatalogId {
    pub fn new<I, S>(connection_id: Uuid, kind: CatalogKind, native_path: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            connection_id,
            kind,
            native_path: native_path.into_iter().map(Into::into).collect(),
        }
    }

    pub const fn profile_id(&self) -> Uuid {
        self.connection_id
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectGroup {
    Tables,
    Views,
    MaterializedViews,
    Sequences,
    Functions,
    Procedures,
    Types,
    Triggers,
}

impl ObjectGroup {
    pub const fn contains_kind(self, kind: CatalogKind) -> bool {
        matches!(
            (self, kind),
            (Self::Tables, CatalogKind::Table)
                | (Self::Views, CatalogKind::View)
                | (Self::MaterializedViews, CatalogKind::MaterializedView)
                | (Self::Sequences, CatalogKind::Sequence)
                | (Self::Functions, CatalogKind::Function)
                | (Self::Procedures, CatalogKind::Procedure)
                | (Self::Types, CatalogKind::Type)
                | (Self::Triggers, CatalogKind::Trigger)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CatalogCount {
    Exact(u64),
    AtLeast(u64),
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogGroupSummary {
    pub group: ObjectGroup,
    pub object_count: CatalogCount,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum OptionalMetadata<T> {
    #[default]
    Unsupported,
    Supported(Option<T>),
}

impl<T> OptionalMetadata<T> {
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported(_))
    }

    pub fn as_ref(&self) -> OptionalMetadata<&T> {
        match self {
            Self::Unsupported => OptionalMetadata::Unsupported,
            Self::Supported(value) => OptionalMetadata::Supported(value.as_ref()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct QualifiedName {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub object: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ColumnMetadataCapabilities {
    pub type_family: bool,
    pub default_expression: bool,
    pub identity: bool,
    pub auto_increment: bool,
    pub generated_expression: bool,
    pub hidden: bool,
    pub numeric_precision_and_scale: bool,
    pub character_length: bool,
    pub collation: bool,
    pub character_set: bool,
    pub comment: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceModel {
    DatabaseAndSchema,
    DatabaseIsSchema,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogCapabilities {
    pub namespace_model: NamespaceModel,
    pub top_level_groups: Vec<ObjectGroup>,
    pub column_metadata: ColumnMetadataCapabilities,
    pub supports_lazy_children: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiscoveredDatabase {
    pub name: String,
    pub schemas: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogDiscovery {
    pub databases: Vec<DiscoveredDatabase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConstraintMembership {
    pub constraint_id: CatalogId,
    pub ordinal_position: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ColumnMetadata {
    pub ordinal_position: u32,
    pub native_type: String,
    pub type_family: OptionalMetadata<String>,
    pub nullable: bool,
    pub default_expression: OptionalMetadata<String>,
    pub identity: OptionalMetadata<bool>,
    pub auto_increment: OptionalMetadata<bool>,
    pub generated_expression: OptionalMetadata<String>,
    pub hidden: OptionalMetadata<bool>,
    pub numeric_precision: OptionalMetadata<u32>,
    pub numeric_scale: OptionalMetadata<u32>,
    pub character_maximum_length: OptionalMetadata<u64>,
    pub collation: OptionalMetadata<String>,
    pub character_set: OptionalMetadata<String>,
    pub constraint_memberships: Vec<ConstraintMembership>,
}

impl ColumnMetadata {
    pub fn new(ordinal_position: u32, native_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            ordinal_position,
            native_type: native_type.into(),
            type_family: OptionalMetadata::Unsupported,
            nullable,
            default_expression: OptionalMetadata::Unsupported,
            identity: OptionalMetadata::Unsupported,
            auto_increment: OptionalMetadata::Unsupported,
            generated_expression: OptionalMetadata::Unsupported,
            hidden: OptionalMetadata::Unsupported,
            numeric_precision: OptionalMetadata::Unsupported,
            numeric_scale: OptionalMetadata::Unsupported,
            character_maximum_length: OptionalMetadata::Unsupported,
            collation: OptionalMetadata::Unsupported,
            character_set: OptionalMetadata::Unsupported,
            constraint_memberships: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexMetadata {
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConstraintMetadata {
    PrimaryKey {
        columns: Vec<String>,
    },
    Unique {
        columns: Vec<String>,
    },
    ForeignKey {
        columns: Vec<String>,
        referenced_relation: QualifiedName,
        referenced_columns: Vec<String>,
    },
    Check {
        expression: String,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CatalogMetadata {
    #[default]
    None,
    Column(ColumnMetadata),
    Index(IndexMetadata),
    Constraint(ConstraintMetadata),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogValidationError {
    #[error("expected {expected}, found catalog kind {found:?}")]
    InvalidKind {
        expected: &'static str,
        found: CatalogKind,
    },
    #[error(
        "catalog child profile {child_profile_id} does not match parent profile {parent_profile_id}"
    )]
    ProfileMismatch {
        child_profile_id: Uuid,
        parent_profile_id: Uuid,
    },
    #[error("qualified catalog object name cannot be empty")]
    EmptyQualifiedObject,
    #[error("metadata does not match catalog kind {kind:?}")]
    MetadataKindMismatch { kind: CatalogKind },
    #[error("foreign-key source and referenced column counts differ")]
    ForeignKeyColumnCountMismatch,
    #[error("catalog page size {found} is outside 1..={max}")]
    InvalidPageSize { found: usize, max: usize },
    #[error(transparent)]
    InvalidScope(#[from] CatalogScopeValidationError),
    #[error("catalog page key does not match its request")]
    RequestKeyMismatch {
        expected: Box<CatalogRequestKey>,
        found: Box<CatalogRequestKey>,
    },
    #[error("catalog page has {found} entries but the request permits {max}")]
    TooManyPageEntries { found: usize, max: usize },
    #[error("catalog page contains duplicate entry {id:?}")]
    DuplicatePageEntry { id: CatalogId },
    #[error("catalog entry {id:?} declares kind {entry_kind:?} but its ID declares {id_kind:?}")]
    EntryKindMismatch {
        id: CatalogId,
        id_kind: CatalogKind,
        entry_kind: CatalogKind,
    },
    #[error("catalog entry {id:?} does not belong to target {target:?}")]
    EntryOutsideTarget {
        id: CatalogId,
        target: CatalogTarget,
    },
    #[error("catalog entry {id:?} is outside the requested catalog scope")]
    EntryOutsideScope { id: CatalogId },
    #[error("catalog entry {id:?} has no native {part} name")]
    MissingQualifiedNamePart { id: CatalogId, part: &'static str },
    #[error("catalog entry {id:?} has a qualified name inconsistent with its kind")]
    InvalidQualifiedNameShape { id: CatalogId },
    #[error("catalog page completeness {found:?} should be {expected:?}")]
    CompletenessMismatch {
        expected: CatalogCompleteness,
        found: CatalogCompleteness,
    },
    #[error("catalog cursor cannot be empty")]
    EmptyCursor,
    #[error("catalog ID {id:?} has an empty native path")]
    EmptyNativePath { id: CatalogId },
    #[error("catalog entry {id:?} has an invalid parent or relation owner")]
    InvalidEntryShape { id: CatalogId },
    #[error("catalog ID {id:?} must have path shape {expected}")]
    InvalidNativePathShape {
        id: CatalogId,
        expected: &'static str,
    },
    #[error("catalog target is outside the requested scope")]
    TargetOutsideScope { target: Box<CatalogTarget> },
    #[error("catalog entry {id:?} native namespace does not match its qualified name")]
    NativeNamespaceMismatch { id: CatalogId },
    #[error("catalog page payload does not match target {target:?}")]
    PagePayloadMismatch { target: Box<CatalogTarget> },
    #[error("catalog page contains duplicate group summary {group:?}")]
    DuplicateGroupSummary { group: ObjectGroup },
    #[error("catalog cursor is not a valid versioned keyset cursor")]
    MalformedCursor,
    #[error("catalog cursor stable native tie-breaker cannot be empty")]
    EmptyCursorTieBreaker,
    #[error("catalog count {count} is smaller than active payload length {payload_len}")]
    TotalCountBelowPayload { count: u64, payload_len: usize },
    #[error("catalog next cursor does not advance beyond the request cursor")]
    NonAdvancingCursor,
    #[error("partial catalog page has {found} active items but page size is {expected}")]
    PartialPagePayloadSizeMismatch { expected: usize, found: usize },
    #[error(
        "initial catalog page exact count {count} is inconsistent with active payload length {payload_len} and continuation state"
    )]
    InitialExactCountMismatch { count: u64, payload_len: usize },
    #[error("catalog ID {id:?} does not preserve its owning native identity")]
    NativeIdentityMismatch { id: CatalogId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    pub id: CatalogId,
    pub parent_id: Option<CatalogId>,
    pub kind: CatalogKind,
    pub native_kind: String,
    pub qualified_name: QualifiedName,
    pub comment: OptionalMetadata<String>,
    pub metadata: CatalogMetadata,
    pub expandable: bool,
    pub relation_id: Option<CatalogId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DdlProvenance {
    NativeCatalog,
    AdapterGenerated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Ddl {
    pub sql: Option<String>,
    pub provenance: DdlProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationStructure {
    pub relation: CatalogEntry,
    pub children: CatalogPage,
    pub ddl: Ddl,
}

#[derive(Default)]
struct CatalogEntryShape {
    parent_id: Option<CatalogId>,
    metadata: CatalogMetadata,
    expandable: bool,
    relation_id: Option<CatalogId>,
}

impl CatalogEntry {
    pub fn database(
        id: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        expandable: bool,
    ) -> Result<Self, CatalogValidationError> {
        require_kind(id.kind, CatalogKind::Database, "database")?;
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                expandable,
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn schema(
        id: CatalogId,
        database: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        expandable: bool,
    ) -> Result<Self, CatalogValidationError> {
        require_kind(id.kind, CatalogKind::Schema, "schema")?;
        require_kind(database.kind, CatalogKind::Database, "database parent")?;
        require_same_profile(&id, &database)?;
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                parent_id: Some(database),
                expandable,
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn relation(
        id: CatalogId,
        schema: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        expandable: bool,
    ) -> Result<Self, CatalogValidationError> {
        require_relation(id.kind, "relation")?;
        require_kind(schema.kind, CatalogKind::Schema, "schema parent")?;
        require_same_profile(&id, &schema)?;
        let relation_id = id.clone();
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                parent_id: Some(schema),
                expandable,
                relation_id: Some(relation_id),
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn object(
        id: CatalogId,
        schema: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        expandable: bool,
    ) -> Result<Self, CatalogValidationError> {
        if !matches!(
            id.kind,
            CatalogKind::Function
                | CatalogKind::Procedure
                | CatalogKind::Sequence
                | CatalogKind::Type
        ) {
            return Err(CatalogValidationError::InvalidKind {
                expected: "schema-level non-relation object",
                found: id.kind,
            });
        }
        require_kind(schema.kind, CatalogKind::Schema, "schema parent")?;
        require_same_profile(&id, &schema)?;
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                parent_id: Some(schema),
                expandable,
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn relation_object(
        id: CatalogId,
        schema: CatalogId,
        relation: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
    ) -> Result<Self, CatalogValidationError> {
        require_kind(
            id.kind,
            CatalogKind::Trigger,
            "relation-owned schema object",
        )?;
        require_kind(schema.kind, CatalogKind::Schema, "schema parent")?;
        require_relation(relation.kind, "owning relation")?;
        require_same_profile(&id, &schema)?;
        require_same_profile(&id, &relation)?;
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                parent_id: Some(schema),
                relation_id: Some(relation),
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn relation_child(
        id: CatalogId,
        relation: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        metadata: CatalogMetadata,
    ) -> Result<Self, CatalogValidationError> {
        if !id.kind.is_relation_child() {
            return Err(CatalogValidationError::InvalidKind {
                expected: "relation child",
                found: id.kind,
            });
        }
        require_relation(relation.kind, "owning relation")?;
        require_same_profile(&id, &relation)?;
        validate_metadata(id.kind, &metadata)?;
        validate_metadata_profiles(&id, &metadata)?;
        Self::new(
            id,
            qualified_name,
            native_kind,
            comment,
            CatalogEntryShape {
                parent_id: Some(relation.clone()),
                metadata,
                relation_id: Some(relation),
                ..CatalogEntryShape::default()
            },
        )
    }

    pub fn owning_relation_id(&self) -> Option<&CatalogId> {
        self.relation_id.as_ref()
    }

    fn new(
        id: CatalogId,
        qualified_name: QualifiedName,
        native_kind: impl Into<String>,
        comment: OptionalMetadata<String>,
        shape: CatalogEntryShape,
    ) -> Result<Self, CatalogValidationError> {
        if qualified_name.object.is_empty() {
            return Err(CatalogValidationError::EmptyQualifiedObject);
        }
        Ok(Self {
            kind: id.kind,
            id,
            parent_id: shape.parent_id,
            native_kind: native_kind.into(),
            qualified_name,
            comment,
            metadata: shape.metadata,
            expandable: shape.expandable,
            relation_id: shape.relation_id,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogTarget {
    Databases,
    Schemas {
        database: CatalogId,
    },
    Groups {
        schema: CatalogId,
    },
    Objects {
        schema: CatalogId,
        group: ObjectGroup,
    },
    RelationChildren {
        relation: CatalogId,
    },
}

impl CatalogTarget {
    pub fn schemas(database: CatalogId) -> Result<Self, CatalogValidationError> {
        require_kind(database.kind, CatalogKind::Database, "database target")?;
        Ok(Self::Schemas { database })
    }

    pub fn groups(schema: CatalogId) -> Result<Self, CatalogValidationError> {
        require_kind(schema.kind, CatalogKind::Schema, "schema target")?;
        Ok(Self::Groups { schema })
    }

    pub fn objects(schema: CatalogId, group: ObjectGroup) -> Result<Self, CatalogValidationError> {
        require_kind(schema.kind, CatalogKind::Schema, "schema target")?;
        Ok(Self::Objects { schema, group })
    }

    pub fn relation_children(relation: CatalogId) -> Result<Self, CatalogValidationError> {
        require_relation(relation.kind, "relation target")?;
        Ok(Self::RelationChildren { relation })
    }

    pub const fn profile_id(&self) -> Option<Uuid> {
        match self {
            Self::Databases => None,
            Self::Schemas { database } => Some(database.profile_id()),
            Self::Groups { schema } | Self::Objects { schema, .. } => Some(schema.profile_id()),
            Self::RelationChildren { relation } => Some(relation.profile_id()),
        }
    }

    pub const fn description(&self) -> &'static str {
        match self {
            Self::Databases => "databases",
            Self::Schemas { .. } => "schemas",
            Self::Groups { .. } => "groups",
            Self::Objects { .. } => "objects",
            Self::RelationChildren { .. } => "relation children",
        }
    }

    fn validate_for_request(
        &self,
        profile_id: Uuid,
        scope: &CatalogScope,
    ) -> Result<(), CatalogValidationError> {
        match self {
            Self::Databases => Ok(()),
            Self::Schemas { database } => {
                require_kind(database.kind, CatalogKind::Database, "database target")?;
                require_profile(database, profile_id)?;
                let database_name = database_path(database)?;
                require_target_scope(self, scope.allows_database(database_name))
            }
            Self::Groups { schema } | Self::Objects { schema, .. } => {
                require_kind(schema.kind, CatalogKind::Schema, "schema target")?;
                require_profile(schema, profile_id)?;
                let (database_name, schema_name) = schema_path(schema)?;
                require_target_scope(self, scope.allows_schema(database_name, schema_name))
            }
            Self::RelationChildren { relation } => {
                require_relation(relation.kind, "relation target")?;
                require_profile(relation, profile_id)?;
                let (database_name, schema_name) = relation_target_path(relation)?;
                require_target_scope(self, scope.allows_schema(database_name, schema_name))
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CatalogCursor(String);

impl CatalogCursor {
    const KEYSET_PREFIX: &'static str = "v1:";

    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_keyset(
        normalized_sort_key: &str,
        native_tie_breaker: &str,
    ) -> Result<Self, CatalogValidationError> {
        if native_tie_breaker.is_empty() {
            return Err(CatalogValidationError::EmptyCursorTieBreaker);
        }
        let cursor = Self(format!(
            "{}{}:{}:{normalized_sort_key}{native_tie_breaker}",
            Self::KEYSET_PREFIX,
            normalized_sort_key.len(),
            native_tie_breaker.len()
        ));
        cursor.keyset_parts()?;
        Ok(cursor)
    }

    pub fn keyset_parts(&self) -> Result<(&str, &str), CatalogValidationError> {
        let encoded = self
            .0
            .strip_prefix(Self::KEYSET_PREFIX)
            .ok_or(CatalogValidationError::MalformedCursor)?;
        let (sort_length, encoded) = encoded
            .split_once(':')
            .ok_or(CatalogValidationError::MalformedCursor)?;
        let (tie_length, payload) = encoded
            .split_once(':')
            .ok_or(CatalogValidationError::MalformedCursor)?;
        let sort_length = sort_length
            .parse::<usize>()
            .map_err(|_| CatalogValidationError::MalformedCursor)?;
        let tie_length = tie_length
            .parse::<usize>()
            .map_err(|_| CatalogValidationError::MalformedCursor)?;
        if tie_length == 0 {
            return Err(CatalogValidationError::EmptyCursorTieBreaker);
        }
        let payload_length = sort_length
            .checked_add(tie_length)
            .ok_or(CatalogValidationError::MalformedCursor)?;
        if payload.len() != payload_length || !payload.is_char_boundary(sort_length) {
            return Err(CatalogValidationError::MalformedCursor);
        }
        let (normalized_sort_key, native_tie_breaker) = payload.split_at(sort_length);
        Ok((normalized_sort_key, native_tie_breaker))
    }
}

impl From<String> for CatalogCursor {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for CatalogCursor {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CatalogRequestKey {
    pub connection: ConnectionIdentity,
    pub catalog_epoch: u64,
    pub request_id: u64,
    pub target: CatalogTarget,
    pub cursor: Option<CatalogCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogRequest {
    pub key: CatalogRequestKey,
    pub scope: CatalogScope,
    pub page_size: usize,
}

pub const MAX_CATALOG_PAGE_SIZE: usize = 500;

impl CatalogRequest {
    pub fn validate(&self) -> Result<(), CatalogValidationError> {
        validate_page_size(self.page_size)?;
        self.scope.validate("", None)?;
        self.key
            .target
            .validate_for_request(self.key.connection.profile_id, &self.scope)?;
        validate_cursor(self.key.cursor.as_ref())
    }

    pub fn validate_for_profile(&self, profile_id: Uuid) -> Result<(), CatalogValidationError> {
        self.validate()?;
        if self.key.connection.profile_id == profile_id {
            Ok(())
        } else {
            Err(CatalogValidationError::ProfileMismatch {
                child_profile_id: self.key.connection.profile_id,
                parent_profile_id: profile_id,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCompleteness {
    Partial,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogPage {
    pub key: CatalogRequestKey,
    pub entries: Vec<CatalogEntry>,
    pub group_summaries: Vec<CatalogGroupSummary>,
    pub total_count: CatalogCount,
    pub next_cursor: Option<CatalogCursor>,
    pub completeness: CatalogCompleteness,
}

impl CatalogPage {
    pub fn new(
        request: &CatalogRequest,
        entries: Vec<CatalogEntry>,
        total_count: CatalogCount,
        next_cursor: Option<CatalogCursor>,
    ) -> Result<Self, CatalogValidationError> {
        let completeness = completeness_for(next_cursor.as_ref());
        let page = Self {
            key: request.key.clone(),
            entries,
            group_summaries: Vec::new(),
            total_count,
            next_cursor,
            completeness,
        };
        page.validate_for(request)?;
        Ok(page)
    }

    pub fn groups(
        request: &CatalogRequest,
        group_summaries: Vec<CatalogGroupSummary>,
        total_count: CatalogCount,
        next_cursor: Option<CatalogCursor>,
    ) -> Result<Self, CatalogValidationError> {
        if !matches!(request.key.target, CatalogTarget::Groups { .. }) {
            return Err(CatalogValidationError::PagePayloadMismatch {
                target: Box::new(request.key.target.clone()),
            });
        }
        let completeness = completeness_for(next_cursor.as_ref());
        let page = Self {
            key: request.key.clone(),
            entries: Vec::new(),
            group_summaries,
            total_count,
            next_cursor,
            completeness,
        };
        page.validate_for(request)?;
        Ok(page)
    }

    pub fn validate_for(&self, request: &CatalogRequest) -> Result<(), CatalogValidationError> {
        request.validate()?;
        if self.key != request.key {
            return Err(CatalogValidationError::RequestKeyMismatch {
                expected: Box::new(request.key.clone()),
                found: Box::new(self.key.clone()),
            });
        }
        validate_cursor(self.next_cursor.as_ref())?;
        let expected_completeness = completeness_for(self.next_cursor.as_ref());
        if self.completeness != expected_completeness {
            return Err(CatalogValidationError::CompletenessMismatch {
                expected: expected_completeness,
                found: self.completeness,
            });
        }
        let active_payload_len = match &request.key.target {
            CatalogTarget::Groups { .. } => {
                if !self.entries.is_empty() {
                    return Err(CatalogValidationError::PagePayloadMismatch {
                        target: Box::new(request.key.target.clone()),
                    });
                }
                let mut groups = HashSet::with_capacity(self.group_summaries.len());
                for summary in &self.group_summaries {
                    if !groups.insert(summary.group) {
                        return Err(CatalogValidationError::DuplicateGroupSummary {
                            group: summary.group,
                        });
                    }
                }
                self.group_summaries.len()
            }
            _ => {
                if !self.group_summaries.is_empty() {
                    return Err(CatalogValidationError::PagePayloadMismatch {
                        target: Box::new(request.key.target.clone()),
                    });
                }
                self.entries.len()
            }
        };
        if active_payload_len > request.page_size {
            return Err(CatalogValidationError::TooManyPageEntries {
                found: active_payload_len,
                max: request.page_size,
            });
        }
        validate_total_count(self.total_count, active_payload_len)?;
        validate_page_progress(
            request,
            self.total_count,
            self.next_cursor.as_ref(),
            active_payload_len,
        )?;

        if !matches!(request.key.target, CatalogTarget::Groups { .. }) {
            let mut entry_ids = HashSet::with_capacity(self.entries.len());
            for entry in &self.entries {
                if !entry_ids.insert(&entry.id) {
                    return Err(CatalogValidationError::DuplicatePageEntry {
                        id: entry.id.clone(),
                    });
                }
                validate_page_entry(entry, request)?;
            }
        }
        Ok(())
    }
}

pub fn finalize_keyset_page<T, SortKey, TieBreaker>(
    rows: &mut Vec<T>,
    page_size: usize,
    normalized_sort_key: SortKey,
    native_tie_breaker: TieBreaker,
) -> Result<Option<CatalogCursor>, CatalogValidationError>
where
    SortKey: Fn(&T) -> String,
    TieBreaker: Fn(&T) -> String,
{
    validate_page_size(page_size)?;
    if rows.len() <= page_size {
        return Ok(None);
    }

    let Some(last) = rows.get(page_size.saturating_sub(1)) else {
        return Ok(None);
    };
    let cursor = CatalogCursor::from_keyset(&normalized_sort_key(last), &native_tie_breaker(last))?;
    rows.truncate(page_size);
    Ok(Some(cursor))
}

fn completeness_for(next_cursor: Option<&CatalogCursor>) -> CatalogCompleteness {
    if next_cursor.is_some() {
        CatalogCompleteness::Partial
    } else {
        CatalogCompleteness::Complete
    }
}

fn validate_page_size(page_size: usize) -> Result<(), CatalogValidationError> {
    if (1..=MAX_CATALOG_PAGE_SIZE).contains(&page_size) {
        Ok(())
    } else {
        Err(CatalogValidationError::InvalidPageSize {
            found: page_size,
            max: MAX_CATALOG_PAGE_SIZE,
        })
    }
}

fn validate_total_count(
    total_count: CatalogCount,
    active_payload_len: usize,
) -> Result<(), CatalogValidationError> {
    let count = match total_count {
        CatalogCount::Exact(count) | CatalogCount::AtLeast(count) => count,
        CatalogCount::Unknown => return Ok(()),
    };
    if count < active_payload_len as u64 {
        Err(CatalogValidationError::TotalCountBelowPayload {
            count,
            payload_len: active_payload_len,
        })
    } else {
        Ok(())
    }
}

fn validate_page_progress(
    request: &CatalogRequest,
    total_count: CatalogCount,
    next_cursor: Option<&CatalogCursor>,
    active_payload_len: usize,
) -> Result<(), CatalogValidationError> {
    if let Some(next_cursor) = next_cursor {
        if let Some(request_cursor) = request.key.cursor.as_ref() {
            let request_parts = request_cursor.keyset_parts()?;
            let next_parts = next_cursor.keyset_parts()?;
            if next_parts <= request_parts {
                return Err(CatalogValidationError::NonAdvancingCursor);
            }
        }
        if active_payload_len != request.page_size {
            return Err(CatalogValidationError::PartialPagePayloadSizeMismatch {
                expected: request.page_size,
                found: active_payload_len,
            });
        }
    }

    if request.key.cursor.is_none()
        && let CatalogCount::Exact(count) = total_count
    {
        let payload_len = active_payload_len as u64;
        let is_consistent = if next_cursor.is_some() {
            count > payload_len
        } else {
            count == payload_len
        };
        if !is_consistent {
            return Err(CatalogValidationError::InitialExactCountMismatch {
                count,
                payload_len: active_payload_len,
            });
        }
    }
    Ok(())
}

fn validate_cursor(cursor: Option<&CatalogCursor>) -> Result<(), CatalogValidationError> {
    if let Some(cursor) = cursor {
        cursor.keyset_parts()?;
    }
    Ok(())
}

fn validate_page_entry(
    entry: &CatalogEntry,
    request: &CatalogRequest,
) -> Result<(), CatalogValidationError> {
    require_profile(&entry.id, request.key.connection.profile_id)?;
    if entry.id.native_path.is_empty() {
        return Err(CatalogValidationError::EmptyNativePath {
            id: entry.id.clone(),
        });
    }
    if entry.id.kind != entry.kind {
        return Err(CatalogValidationError::EntryKindMismatch {
            id: entry.id.clone(),
            id_kind: entry.id.kind,
            entry_kind: entry.kind,
        });
    }
    if let Some(parent) = entry.parent_id.as_ref() {
        require_profile(parent, request.key.connection.profile_id)?;
    }
    if let Some(relation) = entry.relation_id.as_ref() {
        require_profile(relation, request.key.connection.profile_id)?;
    }
    if !valid_entry_shape(entry) {
        return Err(CatalogValidationError::InvalidEntryShape {
            id: entry.id.clone(),
        });
    }
    if entry.kind.is_relation_child() {
        validate_metadata(entry.kind, &entry.metadata)?;
    } else if entry.metadata != CatalogMetadata::None {
        return Err(CatalogValidationError::MetadataKindMismatch { kind: entry.kind });
    }
    validate_metadata_profiles(&entry.id, &entry.metadata)?;
    validate_entry_scope(entry, &request.scope)?;
    validate_entry_namespace(entry)?;
    if let Some(relation) = entry.relation_id.as_ref() {
        validate_metadata_identity(relation, &entry.metadata)?;
    }

    let belongs = match &request.key.target {
        CatalogTarget::Databases => {
            entry.kind == CatalogKind::Database && entry.parent_id.is_none()
        }
        CatalogTarget::Schemas { database } => {
            entry.kind == CatalogKind::Schema && entry.parent_id.as_ref() == Some(database)
        }
        CatalogTarget::Groups { .. } => false,
        CatalogTarget::Objects { schema, group } => {
            group.contains_kind(entry.kind) && entry.parent_id.as_ref() == Some(schema)
        }
        CatalogTarget::RelationChildren { relation } => {
            entry.kind.is_relation_child()
                && entry.parent_id.as_ref() == Some(relation)
                && entry.relation_id.as_ref() == Some(relation)
        }
    };
    if belongs {
        Ok(())
    } else {
        Err(CatalogValidationError::EntryOutsideTarget {
            id: entry.id.clone(),
            target: request.key.target.clone(),
        })
    }
}

fn valid_entry_shape(entry: &CatalogEntry) -> bool {
    match entry.kind {
        CatalogKind::Database => entry.parent_id.is_none() && entry.relation_id.is_none(),
        CatalogKind::Schema => {
            entry
                .parent_id
                .as_ref()
                .is_some_and(|parent| parent.kind == CatalogKind::Database)
                && entry.relation_id.is_none()
        }
        CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView => {
            entry
                .parent_id
                .as_ref()
                .is_some_and(|parent| parent.kind == CatalogKind::Schema)
                && entry.relation_id.as_ref() == Some(&entry.id)
        }
        CatalogKind::Function
        | CatalogKind::Procedure
        | CatalogKind::Sequence
        | CatalogKind::Type => {
            entry
                .parent_id
                .as_ref()
                .is_some_and(|parent| parent.kind == CatalogKind::Schema)
                && entry.relation_id.is_none()
        }
        CatalogKind::Trigger => {
            let Some(parent) = entry.parent_id.as_ref() else {
                return false;
            };
            let Some(relation) = entry.relation_id.as_ref() else {
                return false;
            };
            relation.kind.is_relation()
                && (parent.kind == CatalogKind::Schema || parent == relation)
        }
        CatalogKind::Column
        | CatalogKind::Index
        | CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint => {
            let Some(parent) = entry.parent_id.as_ref() else {
                return false;
            };
            parent.kind.is_relation() && entry.relation_id.as_ref() == Some(parent)
        }
    }
}

fn validate_entry_scope(
    entry: &CatalogEntry,
    scope: &CatalogScope,
) -> Result<(), CatalogValidationError> {
    if entry.qualified_name.object.is_empty() {
        return Err(CatalogValidationError::EmptyQualifiedObject);
    }
    let database = entry.qualified_name.database.as_deref().ok_or_else(|| {
        CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "database",
        }
    })?;
    if database.is_empty() {
        return Err(CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "database",
        });
    }
    if !scope.allows_database(database) {
        return Err(CatalogValidationError::EntryOutsideScope {
            id: entry.id.clone(),
        });
    }

    if entry.kind == CatalogKind::Database {
        if entry.qualified_name.schema.is_some() || entry.qualified_name.object != database {
            return Err(CatalogValidationError::InvalidQualifiedNameShape {
                id: entry.id.clone(),
            });
        }
        return Ok(());
    }

    let schema = entry.qualified_name.schema.as_deref().ok_or_else(|| {
        CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "schema",
        }
    })?;
    if schema.is_empty() {
        return Err(CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "schema",
        });
    }
    if !scope.allows_schema(database, schema) {
        return Err(CatalogValidationError::EntryOutsideScope {
            id: entry.id.clone(),
        });
    }
    if entry.kind == CatalogKind::Schema && entry.qualified_name.object != schema {
        return Err(CatalogValidationError::InvalidQualifiedNameShape {
            id: entry.id.clone(),
        });
    }
    Ok(())
}

fn validate_entry_namespace(entry: &CatalogEntry) -> Result<(), CatalogValidationError> {
    let database = entry.qualified_name.database.as_deref().ok_or_else(|| {
        CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "database",
        }
    })?;
    if entry.kind == CatalogKind::Database {
        if database_path(&entry.id)? == database {
            return Ok(());
        }
        return Err(CatalogValidationError::NativeNamespaceMismatch {
            id: entry.id.clone(),
        });
    }

    let schema = entry.qualified_name.schema.as_deref().ok_or_else(|| {
        CatalogValidationError::MissingQualifiedNamePart {
            id: entry.id.clone(),
            part: "schema",
        }
    })?;
    let expected = (database, schema);
    let entry_namespace = if entry.kind == CatalogKind::Schema {
        schema_path(&entry.id)?
    } else {
        let (native_database, native_schema, native_object) = object_path(&entry.id)?;
        if entry
            .parent_id
            .as_ref()
            .is_some_and(|parent| parent.kind == CatalogKind::Schema)
            && native_object != entry.qualified_name.object
        {
            return Err(CatalogValidationError::NativeIdentityMismatch {
                id: entry.id.clone(),
            });
        }
        (native_database, native_schema)
    };
    if entry_namespace != expected {
        return Err(CatalogValidationError::NativeNamespaceMismatch {
            id: entry.id.clone(),
        });
    }

    if let Some(parent) = entry.parent_id.as_ref() {
        let parent_matches = match parent.kind {
            CatalogKind::Database => database_path(parent)? == database,
            CatalogKind::Schema => schema_path(parent)? == expected,
            kind if kind.is_relation() => object_namespace(parent)? == expected,
            _ => false,
        };
        if !parent_matches {
            return Err(CatalogValidationError::NativeNamespaceMismatch {
                id: entry.id.clone(),
            });
        }
    }

    if let Some(relation) = entry.relation_id.as_ref()
        && (!relation.kind.is_relation() || object_namespace(relation)? != expected)
    {
        return Err(CatalogValidationError::NativeNamespaceMismatch {
            id: entry.id.clone(),
        });
    }
    if let Some(parent) = entry.parent_id.as_ref()
        && parent.kind.is_relation()
        && !strictly_extends_native_path(&entry.id, parent)
    {
        return Err(CatalogValidationError::NativeIdentityMismatch {
            id: entry.id.clone(),
        });
    }
    Ok(())
}

fn require_kind(
    found: CatalogKind,
    expected: CatalogKind,
    expected_description: &'static str,
) -> Result<(), CatalogValidationError> {
    if found == expected {
        Ok(())
    } else {
        Err(CatalogValidationError::InvalidKind {
            expected: expected_description,
            found,
        })
    }
}

fn require_relation(
    found: CatalogKind,
    expected_description: &'static str,
) -> Result<(), CatalogValidationError> {
    if found.is_relation() {
        Ok(())
    } else {
        Err(CatalogValidationError::InvalidKind {
            expected: expected_description,
            found,
        })
    }
}

fn require_same_profile(
    child: &CatalogId,
    parent: &CatalogId,
) -> Result<(), CatalogValidationError> {
    if child.profile_id() == parent.profile_id() {
        Ok(())
    } else {
        Err(CatalogValidationError::ProfileMismatch {
            child_profile_id: child.profile_id(),
            parent_profile_id: parent.profile_id(),
        })
    }
}

fn require_profile(id: &CatalogId, profile_id: Uuid) -> Result<(), CatalogValidationError> {
    if id.profile_id() == profile_id {
        Ok(())
    } else {
        Err(CatalogValidationError::ProfileMismatch {
            child_profile_id: id.profile_id(),
            parent_profile_id: profile_id,
        })
    }
}

fn require_target_scope(
    target: &CatalogTarget,
    allowed: bool,
) -> Result<(), CatalogValidationError> {
    if allowed {
        Ok(())
    } else {
        Err(CatalogValidationError::TargetOutsideScope {
            target: Box::new(target.clone()),
        })
    }
}

fn database_path(id: &CatalogId) -> Result<&str, CatalogValidationError> {
    match id.native_path.as_slice() {
        [database] if !database.is_empty() => Ok(database),
        _ => Err(CatalogValidationError::InvalidNativePathShape {
            id: id.clone(),
            expected: "[database]",
        }),
    }
}

fn schema_path(id: &CatalogId) -> Result<(&str, &str), CatalogValidationError> {
    match id.native_path.as_slice() {
        [database, schema] if !database.is_empty() && !schema.is_empty() => Ok((database, schema)),
        _ => Err(CatalogValidationError::InvalidNativePathShape {
            id: id.clone(),
            expected: "[database, schema]",
        }),
    }
}

fn relation_target_path(id: &CatalogId) -> Result<(&str, &str), CatalogValidationError> {
    match id.native_path.as_slice() {
        [database, schema, object, ..]
            if !database.is_empty() && !schema.is_empty() && !object.is_empty() =>
        {
            Ok((database, schema))
        }
        _ => Err(CatalogValidationError::InvalidNativePathShape {
            id: id.clone(),
            expected: "at least [database, schema, object]",
        }),
    }
}

fn object_namespace(id: &CatalogId) -> Result<(&str, &str), CatalogValidationError> {
    let (database, schema, _) = object_path(id)?;
    Ok((database, schema))
}

fn object_path(id: &CatalogId) -> Result<(&str, &str, &str), CatalogValidationError> {
    match id.native_path.as_slice() {
        [database, schema, object, ..]
            if !database.is_empty() && !schema.is_empty() && !object.is_empty() =>
        {
            Ok((database, schema, object))
        }
        _ => Err(CatalogValidationError::InvalidNativePathShape {
            id: id.clone(),
            expected: "at least [database, schema, object]",
        }),
    }
}

fn strictly_extends_native_path(child: &CatalogId, parent: &CatalogId) -> bool {
    child.native_path.len() > parent.native_path.len()
        && child.native_path.starts_with(&parent.native_path)
}

fn validate_metadata(
    kind: CatalogKind,
    metadata: &CatalogMetadata,
) -> Result<(), CatalogValidationError> {
    let matches = matches!(
        (kind, metadata),
        (CatalogKind::Column, CatalogMetadata::Column(_))
            | (CatalogKind::Index, CatalogMetadata::Index(_))
            | (
                CatalogKind::PrimaryKey,
                CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { .. })
            )
            | (
                CatalogKind::UniqueConstraint,
                CatalogMetadata::Constraint(ConstraintMetadata::Unique { .. })
            )
            | (
                CatalogKind::ForeignKey,
                CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey { .. })
            )
            | (
                CatalogKind::CheckConstraint,
                CatalogMetadata::Constraint(ConstraintMetadata::Check { .. })
            )
            | (CatalogKind::Trigger, CatalogMetadata::None)
    );
    if !matches {
        return Err(CatalogValidationError::MetadataKindMismatch { kind });
    }
    if let CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
        columns,
        referenced_columns,
        ..
    }) = metadata
        && columns.len() != referenced_columns.len()
    {
        return Err(CatalogValidationError::ForeignKeyColumnCountMismatch);
    }
    Ok(())
}

fn validate_metadata_profiles(
    entry: &CatalogId,
    metadata: &CatalogMetadata,
) -> Result<(), CatalogValidationError> {
    let CatalogMetadata::Column(column) = metadata else {
        return Ok(());
    };
    for membership in &column.constraint_memberships {
        require_same_profile(entry, &membership.constraint_id)?;
        if !matches!(
            membership.constraint_id.kind,
            CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint | CatalogKind::ForeignKey
        ) {
            return Err(CatalogValidationError::InvalidKind {
                expected: "column constraint membership",
                found: membership.constraint_id.kind,
            });
        }
    }
    Ok(())
}

fn validate_metadata_identity(
    relation: &CatalogId,
    metadata: &CatalogMetadata,
) -> Result<(), CatalogValidationError> {
    let CatalogMetadata::Column(column) = metadata else {
        return Ok(());
    };
    for membership in &column.constraint_memberships {
        if !strictly_extends_native_path(&membership.constraint_id, relation) {
            return Err(CatalogValidationError::NativeIdentityMismatch {
                id: membership.constraint_id.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Legacy presentation row for the pre-Task-11/12 workspace UI.
/// Runtime catalog state and completion use normalized `CatalogEntry` values.
pub struct CatalogNode {
    pub id: CatalogId,
    pub parent_id: Option<CatalogId>,
    pub kind: CatalogKind,
    pub name: String,
    pub native_kind: String,
    pub detail: Option<String>,
    pub expandable: bool,
}

impl CatalogNode {
    pub fn new(
        id: CatalogId,
        parent_id: Option<CatalogId>,
        name: impl Into<String>,
        native_kind: impl Into<String>,
        detail: Option<String>,
        expandable: bool,
    ) -> Self {
        Self {
            kind: id.kind,
            id,
            parent_id,
            name: name.into(),
            native_kind: native_kind.into(),
            detail,
            expandable,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{CatalogId, CatalogKind};

    #[test]
    fn object_identity_includes_connection_kind_and_native_path() {
        let connection_id = Uuid::new_v4();
        let id = CatalogId::new(
            connection_id,
            CatalogKind::Table,
            ["app", "public", "users"],
        );

        assert_eq!(id.connection_id, connection_id);
        assert_eq!(id.kind, CatalogKind::Table);
        assert_eq!(id.native_path, vec!["app", "public", "users"]);
    }
}
