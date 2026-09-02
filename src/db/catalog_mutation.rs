#![allow(clippy::too_many_arguments)]

use thiserror::Error;
use uuid::Uuid;

use crate::{
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogTarget, ObjectGroup, OptionalMetadata,
    },
    identity::ConnectionIdentity,
    model::execution_target::ExecutionTarget,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogMutationMode {
    Create,
    Edit,
}

/// The database a catalog operation is allowed to use.  A maintenance target
/// is deliberately separate from a console target so maintenance work cannot
/// accidentally rewrite the user's workspace target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogMutationTarget {
    Database(ExecutionTarget),
    Maintenance { database: String },
}

impl CatalogMutationTarget {
    pub fn database_target(target: ExecutionTarget) -> Result<Self, CatalogMutationError> {
        if target.database.trim().is_empty() {
            return Err(CatalogMutationError::InvalidAnchor {
                reason: "catalog mutation target database is empty",
            });
        }
        Ok(Self::Database(target))
    }

    pub fn maintenance(database: impl Into<String>) -> Result<Self, CatalogMutationError> {
        let database = database.into();
        if database.trim().is_empty() {
            return Err(CatalogMutationError::InvalidAnchor {
                reason: "maintenance target database is empty",
            });
        }
        Ok(Self::Maintenance { database })
    }

    pub fn database(&self) -> &str {
        match self {
            Self::Database(target) => &target.database,
            Self::Maintenance { database } => database,
        }
    }

    pub fn execution_target(&self, profile_id: Uuid) -> ExecutionTarget {
        ExecutionTarget {
            profile_id,
            database: self.database().to_owned(),
            schema: match self {
                Self::Database(target) => target.schema.clone(),
                Self::Maintenance { .. } => None,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogMutationAnchor {
    Profile {
        profile_id: Uuid,
    },
    Catalog(CatalogId),
    Group {
        schema: CatalogId,
        group: ObjectGroup,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CatalogObjectType {
    Catalog(CatalogKind),
    LoginRole,
    Role,
}

impl CatalogObjectType {
    pub const fn display_label(self) -> &'static str {
        match self {
            Self::Catalog(CatalogKind::Database) => "Database",
            Self::Catalog(CatalogKind::Schema) => "Schema",
            Self::Catalog(CatalogKind::Table) => "Table",
            Self::Catalog(CatalogKind::View) => "View",
            Self::Catalog(CatalogKind::MaterializedView) => "Materialized View",
            Self::Catalog(CatalogKind::Column) => "Column",
            Self::Catalog(CatalogKind::Index) => "Index",
            Self::Catalog(CatalogKind::PrimaryKey) => "Primary Key",
            Self::Catalog(CatalogKind::UniqueConstraint) => "Unique",
            Self::Catalog(CatalogKind::ForeignKey) => "Foreign Key",
            Self::Catalog(CatalogKind::CheckConstraint) => "Check",
            Self::Catalog(CatalogKind::Function) => "Function",
            Self::Catalog(CatalogKind::Procedure) => "Procedure",
            Self::Catalog(CatalogKind::Trigger) => "Trigger",
            Self::Catalog(CatalogKind::Sequence) => "Sequence",
            Self::Catalog(CatalogKind::Type) => "Type",
            Self::LoginRole => "Login Role",
            Self::Role => "Role",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogMutationAvailability {
    Available,
    Unavailable { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogMutationOption {
    pub object_type: CatalogObjectType,
    pub availability: CatalogMutationAvailability,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogMutationCapabilities {
    pub profile_create: Vec<CatalogMutationOption>,
    pub create: Vec<CatalogMutationOption>,
    pub edit: Vec<CatalogMutationOption>,
    pub view_options: ViewMutationCapabilities,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewMutationOptionAvailability {
    Available,
    Unavailable { reason: &'static str },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewOption<T> {
    pub availability: ViewMutationOptionAvailability,
    pub value: Option<T>,
}

impl<T> ViewOption<T> {
    pub const fn unavailable(reason: &'static str) -> Self {
        Self {
            availability: ViewMutationOptionAvailability::Unavailable { reason },
            value: None,
        }
    }
    pub const fn available(value: Option<T>) -> Self {
        Self {
            availability: ViewMutationOptionAvailability::Available,
            value,
        }
    }
}

impl ViewMutationOptionAvailability {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewMutationCapabilities {
    pub security_barrier: ViewMutationOptionAvailability,
    pub security_invoker: ViewMutationOptionAvailability,
    pub check_option: ViewMutationOptionAvailability,
}

impl Default for ViewMutationCapabilities {
    fn default() -> Self {
        Self {
            security_barrier: ViewMutationOptionAvailability::Unavailable {
                reason: "server version is unknown",
            },
            security_invoker: ViewMutationOptionAvailability::Unavailable {
                reason: "server version is unknown",
            },
            check_option: ViewMutationOptionAvailability::Unavailable {
                reason: "server version is unknown",
            },
        }
    }
}

impl CatalogMutationCapabilities {
    pub fn create_options(
        &self,
        anchor: &CatalogMutationAnchor,
        entry: Option<&CatalogEntry>,
    ) -> Result<Vec<CatalogObjectType>, CatalogMutationError> {
        match anchor {
            CatalogMutationAnchor::Profile { .. } => Ok(self
                .profile_create
                .iter()
                .filter_map(available_object_type)
                .collect()),
            CatalogMutationAnchor::Catalog(id) => {
                if let Some(entry) = entry {
                    validate_entry(anchor, entry)?;
                }
                let object_types = match id.kind {
                    CatalogKind::Database => vec![CatalogObjectType::Catalog(CatalogKind::Schema)],
                    CatalogKind::Schema => vec![
                        CatalogObjectType::Catalog(CatalogKind::Table),
                        CatalogObjectType::Catalog(CatalogKind::View),
                        CatalogObjectType::Catalog(CatalogKind::MaterializedView),
                    ],
                    CatalogKind::Table => vec![
                        CatalogObjectType::Catalog(CatalogKind::Column),
                        CatalogObjectType::Catalog(CatalogKind::Index),
                        CatalogObjectType::Catalog(CatalogKind::PrimaryKey),
                        CatalogObjectType::Catalog(CatalogKind::UniqueConstraint),
                        CatalogObjectType::Catalog(CatalogKind::ForeignKey),
                        CatalogObjectType::Catalog(CatalogKind::CheckConstraint),
                    ],
                    CatalogKind::MaterializedView => {
                        vec![CatalogObjectType::Catalog(CatalogKind::Index)]
                    }
                    _ => Vec::new(),
                };
                Ok(object_types
                    .into_iter()
                    .filter(|object_type| self.is_available(*object_type, false))
                    .collect())
            }
            CatalogMutationAnchor::Group { group, .. } => {
                if let Some(entry) = entry {
                    validate_entry(anchor, entry)?;
                }
                let kind = match group {
                    ObjectGroup::Tables => CatalogKind::Table,
                    ObjectGroup::Views => CatalogKind::View,
                    ObjectGroup::MaterializedViews => CatalogKind::MaterializedView,
                    ObjectGroup::Sequences => CatalogKind::Sequence,
                    ObjectGroup::Functions => CatalogKind::Function,
                    ObjectGroup::Procedures => CatalogKind::Procedure,
                    ObjectGroup::Types => CatalogKind::Type,
                    ObjectGroup::Triggers => CatalogKind::Trigger,
                };
                Ok(self
                    .is_available(CatalogObjectType::Catalog(kind), false)
                    .then_some(CatalogObjectType::Catalog(kind))
                    .into_iter()
                    .collect())
            }
        }
    }

    pub fn can_edit(
        &self,
        anchor: &CatalogMutationAnchor,
        entry: Option<&CatalogEntry>,
    ) -> Result<bool, CatalogMutationError> {
        let CatalogMutationAnchor::Catalog(id) = anchor else {
            return Ok(false);
        };
        let Some(entry) = entry else {
            return Err(CatalogMutationError::EmptySelection);
        };
        validate_entry(anchor, entry)?;
        Ok(self.is_available(CatalogObjectType::Catalog(id.kind), true))
    }

    pub fn create_availability(
        &self,
        object_type: CatalogObjectType,
    ) -> Option<CatalogMutationAvailability> {
        self.option(object_type, false)
            .map(|option| option.availability)
    }

    pub fn edit_availability(
        &self,
        object_type: CatalogObjectType,
    ) -> Option<CatalogMutationAvailability> {
        self.option(object_type, true)
            .map(|option| option.availability)
    }

    fn option(&self, object_type: CatalogObjectType, edit: bool) -> Option<CatalogMutationOption> {
        (if edit { &self.edit } else { &self.create })
            .iter()
            .find(|option| option.object_type == object_type)
            .copied()
    }

    fn is_available(&self, object_type: CatalogObjectType, edit: bool) -> bool {
        matches!(
            self.option(object_type, edit)
                .map(|option| option.availability),
            Some(CatalogMutationAvailability::Available)
        )
    }
}

fn available_object_type(option: &CatalogMutationOption) -> Option<CatalogObjectType> {
    matches!(option.availability, CatalogMutationAvailability::Available)
        .then_some(option.object_type)
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogMutationError {
    #[error("catalog mutation operation {object_type:?} is unsupported")]
    UnsupportedOperation { object_type: CatalogObjectType },
    #[error(
        "catalog mutation profile {object_profile_id} does not match connection profile {connection_profile_id}"
    )]
    ProfileMismatch {
        object_profile_id: Uuid,
        connection_profile_id: Uuid,
    },
    #[error("catalog mutation anchor is invalid: {reason}")]
    InvalidAnchor { reason: &'static str },
    #[error("catalog mutation selection is empty")]
    EmptySelection,
    #[error("catalog mutation state is stale")]
    StaleState,
    #[error("catalog mutation draft is invalid: {reason}")]
    InvalidDraft { reason: String },
    #[error("catalog mutation plan is invalid: {reason}")]
    InvalidPlan { reason: String },
    #[error("catalog mutation has no changes")]
    NoChanges,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutationRequest {
    pub connection: ConnectionIdentity,
    pub request_id: u64,
    pub catalog_epoch: u64,
    pub mode: CatalogMutationMode,
    pub anchor: CatalogMutationAnchor,
    pub object_type: CatalogObjectType,
    pub current_database: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogObjectDefinitionRequest {
    pub connection: ConnectionIdentity,
    pub request_id: u64,
    pub catalog_epoch: u64,
    pub object: CatalogId,
    pub target: ExecutionTarget,
}

impl CatalogObjectDefinitionRequest {
    pub fn is_role(&self) -> bool {
        self.object.kind == CatalogKind::Database
            && self
                .object
                .native_path
                .first()
                .is_some_and(|v| v == "__role__")
    }

    pub fn validate(&self) -> Result<(), CatalogMutationError> {
        if self.is_role() {
            if self.object.native_path.len() != 2 {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "role definition requires a role name",
                });
            }
            validate_object_profile(&self.object, self.connection.profile_id)?;
            if self.target.profile_id != self.connection.profile_id
                || self.target.database.is_empty()
            {
                return Err(CatalogMutationError::ProfileMismatch {
                    object_profile_id: self.target.profile_id,
                    connection_profile_id: self.connection.profile_id,
                });
            }
            return Ok(());
        }
        if !matches!(
            self.object.kind,
            CatalogKind::Database
                | CatalogKind::Schema
                | CatalogKind::Table
                | CatalogKind::Column
                | CatalogKind::Index
                | CatalogKind::PrimaryKey
                | CatalogKind::UniqueConstraint
                | CatalogKind::ForeignKey
                | CatalogKind::CheckConstraint
                | CatalogKind::View
                | CatalogKind::MaterializedView
                | CatalogKind::Sequence
        ) {
            return Err(CatalogMutationError::InvalidAnchor {
                reason: "definition request requires a supported catalog object",
            });
        }
        if self.object.native_path.is_empty() {
            return Err(CatalogMutationError::EmptySelection);
        }
        validate_object_profile(&self.object, self.connection.profile_id)?;
        if self.target.profile_id != self.connection.profile_id || self.target.database.is_empty() {
            return Err(CatalogMutationError::ProfileMismatch {
                object_profile_id: self.target.profile_id,
                connection_profile_id: self.connection.profile_id,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaDefinition {
    pub database: String,
    pub name: String,
    pub owner: String,
    pub comment: OptionalMetadata<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseDefinition {
    pub name: String,
    pub owner: String,
    pub template: String,
    pub encoding: String,
    pub locale_provider: String,
    pub locale: String,
    pub collation: String,
    pub ctype: String,
    pub tablespace: String,
    pub connection_limit: i32,
    pub allow_connections: bool,
    pub is_template: bool,
    pub comment: OptionalMetadata<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleDefinition {
    pub name: String,
    pub login: bool,
    pub superuser: bool,
    pub createdb: bool,
    pub createrole: bool,
    pub inherit: bool,
    pub replication: bool,
    pub bypass_rls: bool,
    pub connection_limit: i32,
    pub valid_until: OptionalMetadata<String>,
    pub memberships: Vec<String>,
    pub comment: OptionalMetadata<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnDefinition {
    pub name: String,
    pub ordinal_position: u32,
    pub native_type: String,
    pub nullable: bool,
    pub default_expression: OptionalMetadata<String>,
    pub identity: OptionalMetadata<bool>,
    pub generated_expression: OptionalMetadata<String>,
    pub collation: OptionalMetadata<String>,
    pub comment: OptionalMetadata<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDefinition {
    pub database: String,
    pub schema: String,
    pub name: String,
    pub owner: String,
    pub comment: OptionalMetadata<String>,
    pub columns: Vec<ColumnDefinition>,
    pub indexes: Vec<String>,
    pub constraints: Vec<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewDefinition {
    pub database: String,
    pub schema: String,
    pub name: String,
    pub owner: String,
    pub comment: OptionalMetadata<String>,
    pub query: String,
    pub output_columns: Vec<String>,
    pub security_barrier: ViewOption<bool>,
    pub security_invoker: ViewOption<bool>,
    pub check_option: ViewOption<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedViewDefinition {
    pub database: String,
    pub schema: String,
    pub name: String,
    pub owner: String,
    pub comment: OptionalMetadata<String>,
    pub query: String,
    pub tablespace: OptionalMetadata<String>,
    pub populated: bool,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SequenceBound {
    Unset,
    Value(String),
    NoLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceDefinition {
    pub database: String,
    pub schema: String,
    pub name: String,
    pub owner: String,
    pub comment: OptionalMetadata<String>,
    pub data_type: String,
    pub increment: String,
    pub min_value: SequenceBound,
    pub max_value: SequenceBound,
    pub start_value: String,
    pub cache: String,
    pub cycle: bool,
    pub owned_by: Option<(String, String, String)>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexColumnDefinition {
    pub expression: String,
    pub descending: bool,
    pub nulls_first: bool,
    pub is_expression: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexDefinition {
    pub database: String,
    pub schema: String,
    pub relation: String,
    pub relation_kind: CatalogKind,
    pub name: String,
    pub unique: bool,
    pub access_method: String,
    pub columns: Vec<IndexColumnDefinition>,
    pub include_columns: Vec<String>,
    pub predicate: OptionalMetadata<String>,
    pub tablespace: OptionalMetadata<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConstraintDefinitionKind {
    PrimaryKey {
        columns: Vec<String>,
    },
    Unique {
        columns: Vec<String>,
    },
    ForeignKey {
        columns: Vec<String>,
        referenced_schema: String,
        referenced_relation: String,
        referenced_columns: Vec<String>,
        match_type: String,
        on_update: String,
        on_delete: String,
    },
    Check {
        expression: String,
        no_inherit: bool,
    },
}

impl ConstraintDefinitionKind {
    pub const fn catalog_kind(&self) -> CatalogKind {
        match self {
            Self::PrimaryKey { .. } => CatalogKind::PrimaryKey,
            Self::Unique { .. } => CatalogKind::UniqueConstraint,
            Self::ForeignKey { .. } => CatalogKind::ForeignKey,
            Self::Check { .. } => CatalogKind::CheckConstraint,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstraintDefinition {
    pub database: String,
    pub schema: String,
    pub relation: String,
    pub relation_kind: CatalogKind,
    pub name: String,
    pub kind: ConstraintDefinitionKind,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub validated: bool,
    pub comment: OptionalMetadata<String>,
    pub baseline_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogObjectDefinition {
    Database(DatabaseDefinition),
    Role(RoleDefinition),
    Schema(SchemaDefinition),
    Table(TableDefinition),
    Index(IndexDefinition),
    Constraint(ConstraintDefinition),
    View(ViewDefinition),
    MaterializedView(MaterializedViewDefinition),
    Sequence(SequenceDefinition),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogMutationExecutionMode {
    Transactional,
    Autocommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogSelectionHint {
    Object(CatalogId),
    Parent(CatalogTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutationNamespace {
    pub database: Option<CatalogId>,
    pub schema: Option<CatalogId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutationImpact {
    pub old_object_id: CatalogId,
    pub owning_relation_id: Option<CatalogId>,
    pub namespace: CatalogMutationNamespace,
    pub native_identity_changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutationPlan {
    pub request: CatalogMutationRequest,
    pub object_type: CatalogObjectType,
    pub execution_mode: CatalogMutationExecutionMode,
    pub refresh: Vec<CatalogTarget>,
    pub selection: CatalogSelectionHint,
    pub baseline_fingerprint: Option<String>,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub impact: CatalogMutationImpact,
    pub execution_target: Option<CatalogMutationTarget>,
    statements: Vec<String>,
    #[allow(dead_code)]
    pub(crate) execution_secret: Option<crate::security::RedactedSecret>,
}

impl CatalogMutationPlan {
    pub fn new(
        request: CatalogMutationRequest,
        object_type: CatalogObjectType,
        execution_mode: CatalogMutationExecutionMode,
        refresh: Vec<CatalogTarget>,
        selection: CatalogSelectionHint,
        baseline_fingerprint: Option<String>,
        warnings: Vec<String>,
        statements: Vec<String>,
    ) -> Result<Self, CatalogMutationError> {
        let old_object_id = match &request.anchor {
            CatalogMutationAnchor::Catalog(id) => id.clone(),
            CatalogMutationAnchor::Group { schema, .. } => schema.clone(),
            CatalogMutationAnchor::Profile { profile_id } => {
                CatalogId::new(*profile_id, CatalogKind::Database, [""])
            }
        };
        let plan = Self {
            request,
            object_type,
            execution_mode,
            refresh,
            selection,
            baseline_fingerprint,
            warnings,
            destructive: false,
            impact: CatalogMutationImpact {
                old_object_id,
                owning_relation_id: None,
                namespace: CatalogMutationNamespace {
                    database: None,
                    schema: None,
                },
                native_identity_changed: false,
            },
            execution_target: None,
            statements,
            execution_secret: None,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), CatalogMutationError> {
        if self.statements.is_empty()
            || self
                .statements
                .iter()
                .any(|statement| statement.trim().is_empty())
        {
            return Err(CatalogMutationError::InvalidPlan {
                reason: "plan must contain non-empty statements".into(),
            });
        }
        if self.request.object_type != self.object_type {
            return Err(CatalogMutationError::InvalidPlan {
                reason: "plan object type does not match request".into(),
            });
        }
        if self.refresh.is_empty() {
            return Err(CatalogMutationError::InvalidPlan {
                reason: "plan must contain a refresh target".into(),
            });
        }
        for target in &self.refresh {
            if target
                .profile_id()
                .is_some_and(|id| id != self.request.connection.profile_id)
            {
                return Err(CatalogMutationError::ProfileMismatch {
                    object_profile_id: target.profile_id().unwrap(),
                    connection_profile_id: self.request.connection.profile_id,
                });
            }
        }
        match &self.selection {
            CatalogSelectionHint::Object(object) => {
                validate_object_profile(object, self.request.connection.profile_id)?;
                if object.kind != self.object_type_kind() {
                    return Err(CatalogMutationError::InvalidPlan {
                        reason: "selection object type does not match plan".into(),
                    });
                }
            }
            CatalogSelectionHint::Parent(target) => {
                if target
                    .profile_id()
                    .is_some_and(|id| id != self.request.connection.profile_id)
                {
                    return Err(CatalogMutationError::InvalidPlan {
                        reason: "selection target profile does not match request".into(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn with_impact(mut self, impact: CatalogMutationImpact) -> Self {
        self.impact = impact;
        self
    }

    pub fn with_destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn with_execution_target(mut self, target: CatalogMutationTarget) -> Self {
        self.execution_target = Some(target);
        self
    }

    fn object_type_kind(&self) -> CatalogKind {
        match self.object_type {
            CatalogObjectType::Catalog(kind) => kind,
            CatalogObjectType::LoginRole | CatalogObjectType::Role => CatalogKind::Database,
        }
    }

    pub fn statements(&self) -> &[String] {
        &self.statements
    }

    pub fn sql(&self) -> String {
        self.statements.join("\n")
    }

    pub(crate) fn with_execution_secret_opt(
        mut self,
        secret: Option<crate::security::RedactedSecret>,
    ) -> Self {
        self.execution_secret = secret;
        self
    }

    pub(crate) fn execution_secret(&self) -> Option<&crate::security::RedactedSecret> {
        self.execution_secret.as_ref()
    }
}

impl CatalogMutationRequest {
    pub fn new(
        connection: ConnectionIdentity,
        request_id: u64,
        catalog_epoch: u64,
        mode: CatalogMutationMode,
        anchor: CatalogMutationAnchor,
        object_type: CatalogObjectType,
    ) -> Result<Self, CatalogMutationError> {
        match &anchor {
            CatalogMutationAnchor::Profile { profile_id } => {
                if *profile_id != connection.profile_id {
                    return Err(CatalogMutationError::ProfileMismatch {
                        object_profile_id: *profile_id,
                        connection_profile_id: connection.profile_id,
                    });
                }
                if !matches!(
                    object_type,
                    CatalogObjectType::LoginRole
                        | CatalogObjectType::Role
                        | CatalogObjectType::Catalog(CatalogKind::Database)
                ) {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "profile anchors only support profile objects",
                    });
                }
            }
            CatalogMutationAnchor::Catalog(object) => {
                validate_object_profile(object, connection.profile_id)?;
                if object.native_path.is_empty() {
                    return Err(CatalogMutationError::EmptySelection);
                }
                if matches!(mode, CatalogMutationMode::Edit)
                    && !matches!(
                        object_type,
                        CatalogObjectType::LoginRole | CatalogObjectType::Role
                    )
                    && object_type != CatalogObjectType::Catalog(object.kind)
                {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "edit object type does not match catalog anchor",
                    });
                }
            }
            CatalogMutationAnchor::Group { schema, group } => {
                validate_object_profile(schema, connection.profile_id)?;
                if schema.native_path.is_empty() || schema.kind != CatalogKind::Schema {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "group anchor requires a schema ID",
                    });
                }
                if !matches!(object_type, CatalogObjectType::Catalog(kind) if group.contains_kind(kind))
                {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "object type is outside the selected group",
                    });
                }
            }
        }
        Ok(Self {
            connection,
            request_id,
            catalog_epoch,
            mode,
            anchor,
            object_type,
            current_database: None,
        })
    }

    pub fn with_current_database(mut self, database: impl Into<String>) -> Self {
        self.current_database = Some(database.into());
        self
    }
}

fn validate_object_profile(
    object: &CatalogId,
    profile_id: Uuid,
) -> Result<(), CatalogMutationError> {
    if object.profile_id() != profile_id {
        return Err(CatalogMutationError::ProfileMismatch {
            object_profile_id: object.profile_id(),
            connection_profile_id: profile_id,
        });
    }
    Ok(())
}

fn validate_entry(
    anchor: &CatalogMutationAnchor,
    entry: &CatalogEntry,
) -> Result<(), CatalogMutationError> {
    match anchor {
        CatalogMutationAnchor::Catalog(id) if entry.id != *id => {
            Err(CatalogMutationError::InvalidAnchor {
                reason: "selected entry does not match catalog anchor",
            })
        }
        CatalogMutationAnchor::Group { schema, group }
            if entry.parent_id.as_ref() != Some(schema) || !group.contains_kind(entry.kind) =>
        {
            Err(CatalogMutationError::InvalidAnchor {
                reason: "selected entry does not belong to group anchor",
            })
        }
        _ => Ok(()),
    }
}
