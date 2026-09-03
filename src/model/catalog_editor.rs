use uuid::Uuid;

use crate::{
    db::{
        catalog::CatalogId,
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogObjectType, CatalogOwnerChoice,
        },
    },
    model::text_input::TextInput,
    security::RedactedSecret,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleDraft {
    pub name: TextInput,
    pub login: bool,
    pub superuser: bool,
    pub createdb: bool,
    pub createrole: bool,
    pub inherit: bool,
    pub replication: bool,
    pub bypass_rls: bool,
    pub connection_limit: TextInput,
    pub password: Option<RedactedSecret>,
    pub valid_until: TextInput,
    pub memberships: TextInput,
    pub comment: TextInput,
    pub selected_field: usize,
}

impl RoleDraft {
    pub fn new(login: bool) -> Self {
        Self {
            name: TextInput::default(),
            login,
            superuser: false,
            createdb: false,
            createrole: false,
            inherit: true,
            replication: false,
            bypass_rls: false,
            connection_limit: "-1".into(),
            password: None,
            valid_until: "infinity".into(),
            memberships: TextInput::default(),
            comment: TextInput::default(),
            selected_field: 0,
        }
    }
    pub fn from_definition(d: &crate::db::catalog_mutation::RoleDefinition) -> Self {
        let mut role = Self::new(d.login);
        role.name = d.name.clone().into();
        role.superuser = d.superuser;
        role.createdb = d.createdb;
        role.createrole = d.createrole;
        role.inherit = d.inherit;
        role.replication = d.replication;
        role.bypass_rls = d.bypass_rls;
        role.connection_limit = d.connection_limit.to_string().into();
        role.valid_until = optional_text(&d.valid_until);
        role.memberships = d.memberships.join(", ").into();
        role.comment = optional_text(&d.comment);
        role
    }
    pub fn set_password(&mut self, value: impl Into<String>) {
        self.password = Some(RedactedSecret::new(value));
    }
    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() {
            return Err(invalid("role name is required"));
        }
        if self.connection_limit.value().trim().parse::<i32>().is_err() {
            return Err(invalid("connection limit must be an integer"));
        }
        Ok(())
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(11) as usize;
    }
    fn input(&mut self) -> Option<&mut TextInput> {
        match self.selected_field {
            0 => Some(&mut self.name),
            8 => Some(&mut self.connection_limit),
            9 => Some(&mut self.valid_until),
            10 => Some(&mut self.comment),
            _ => None,
        }
    }
    pub fn insert(&mut self, c: char) {
        if let Some(i) = self.input() {
            i.insert(c)
        }
    }
    pub fn backspace(&mut self) {
        if let Some(i) = self.input() {
            i.backspace()
        }
    }
    pub fn delete(&mut self) {
        if let Some(i) = self.input() {
            i.delete()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogEditorPage {
    ObjectPicker,
    Loading,
    Form,
    SqlPreview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogEditorOperation {
    LoadingDefinition { request_id: u64 },
    Planning { request_id: u64 },
    Applying { request_id: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogEditorSection {
    General,
    Columns,
    Indexes,
    Constraints,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableEditorField {
    Name,
    Schema,
    Owner,
    Comment,
    ColumnList,
    ColumnName,
    ColumnType,
    ColumnNullable,
    ColumnDefault,
    ColumnIdentity,
    ColumnComment,
    AddColumn,
    RemoveColumn,
    Review,
    Cancel,
}

impl TableEditorField {
    const ALL: [Self; 15] = [
        Self::Name,
        Self::Schema,
        Self::Owner,
        Self::Comment,
        Self::ColumnList,
        Self::ColumnName,
        Self::ColumnType,
        Self::ColumnNullable,
        Self::ColumnDefault,
        Self::ColumnIdentity,
        Self::ColumnComment,
        Self::AddColumn,
        Self::RemoveColumn,
        Self::Review,
        Self::Cancel,
    ];

    fn move_by(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default();
        let index = (index as isize + delta).rem_euclid(Self::ALL.len() as isize) as usize;
        Self::ALL[index]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutationOption {
    pub object_type: CatalogObjectType,
    pub label: String,
}

/// Row index of the owner field inside the schema form.
pub const SCHEMA_OWNER_FIELD: usize = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaDraft {
    pub name: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub selected_field: usize,
}

impl SchemaDraft {
    pub fn new() -> Self {
        Self {
            name: TextInput::default(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            selected_field: 0,
        }
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(3) as usize;
    }

    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.owner,
            _ => &mut self.comment,
        }
    }

    pub fn insert(&mut self, character: char) {
        self.selected_input_mut().insert(character);
    }

    pub fn backspace(&mut self) {
        self.selected_input_mut().backspace();
    }

    pub fn delete(&mut self) {
        self.selected_input_mut().delete();
    }

    pub fn delete_previous_word(&mut self) {
        self.selected_input_mut().delete_previous_word();
    }

    pub fn delete_to_start(&mut self) {
        self.selected_input_mut().delete_to_start();
    }

    pub fn move_left(&mut self) {
        self.selected_input_mut().move_left();
    }

    pub fn move_right(&mut self) {
        self.selected_input_mut().move_right();
    }

    pub fn move_home(&mut self) {
        self.selected_input_mut().move_home();
    }

    pub fn move_end(&mut self) {
        self.selected_input_mut().move_end();
    }
}

impl Default for SchemaDraft {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseDraft {
    pub name: TextInput,
    pub owner: TextInput,
    pub template: TextInput,
    pub encoding: TextInput,
    pub locale_provider: TextInput,
    pub locale: TextInput,
    pub collation: TextInput,
    pub ctype: TextInput,
    pub tablespace: TextInput,
    pub connection_limit: TextInput,
    pub allow_connections: bool,
    pub is_template: bool,
    pub comment: TextInput,
    pub selected_field: usize,
    pub editable_creation_options: bool,
}

impl DatabaseDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::DatabaseDefinition) -> Self {
        Self {
            name: definition.name.clone().into(),
            owner: definition.owner.clone().into(),
            template: definition.template.clone().into(),
            encoding: definition.encoding.clone().into(),
            locale_provider: definition.locale_provider.clone().into(),
            locale: definition.locale.clone().into(),
            collation: definition.collation.clone().into(),
            ctype: definition.ctype.clone().into(),
            tablespace: definition.tablespace.clone().into(),
            connection_limit: definition.connection_limit.to_string().into(),
            allow_connections: definition.allow_connections,
            is_template: definition.is_template,
            comment: optional_text(&definition.comment),
            selected_field: 0,
            editable_creation_options: false,
        }
    }
    pub fn new(_database: impl Into<String>) -> Self {
        Self {
            name: TextInput::default(),
            owner: TextInput::default(),
            template: "template0".into(),
            encoding: "UTF8".into(),
            locale_provider: "libc".into(),
            locale: "C".into(),
            collation: "C".into(),
            ctype: "C".into(),
            tablespace: TextInput::default(),
            connection_limit: "-1".into(),
            allow_connections: true,
            is_template: false,
            comment: TextInput::default(),
            selected_field: 0,
            editable_creation_options: true,
        }
    }
    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.owner.value().trim().is_empty() {
            return Err(invalid("database name and owner are required"));
        }
        if self.connection_limit.value().trim().parse::<i32>().is_err() {
            return Err(invalid("connection limit must be an integer"));
        }
        if self.editable_creation_options
            && (self.template.value().trim().is_empty() || self.encoding.value().trim().is_empty())
        {
            return Err(invalid("database template and encoding are required"));
        }
        Ok(())
    }
    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(13) as usize;
    }
    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.owner,
            2 => &mut self.template,
            3 => &mut self.encoding,
            4 => &mut self.locale_provider,
            5 => &mut self.locale,
            6 => &mut self.collation,
            7 => &mut self.ctype,
            8 => &mut self.tablespace,
            9 => &mut self.connection_limit,
            _ => &mut self.comment,
        }
    }
    pub fn insert(&mut self, c: char) {
        if self.editable_creation_options || !matches!(self.selected_field, 2..=8) {
            self.selected_input_mut().insert(c);
        }
    }
    pub fn backspace(&mut self) {
        if self.editable_creation_options || !matches!(self.selected_field, 2..=8) {
            self.selected_input_mut().backspace();
        }
    }
    pub fn delete(&mut self) {
        if self.editable_creation_options || !matches!(self.selected_field, 2..=8) {
            self.selected_input_mut().delete();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnDraft {
    pub row_id: Uuid,
    pub ordinal_position: u32,
    pub existing_name: Option<String>,
    pub name: TextInput,
    pub native_type: TextInput,
    pub nullable: bool,
    pub default_expression: TextInput,
    pub identity: bool,
    pub generated_expression: TextInput,
    pub collation: TextInput,
    pub comment: TextInput,
    pub state: DraftRowState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftRowState {
    Existing { id: CatalogId },
    Added,
    Removed { id: CatalogId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub columns: Vec<ColumnDraft>,
    pub selected_section: CatalogEditorSection,
    pub selected_column: usize,
    pub selected_field: TableEditorField,
    pub indexes: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub query: TextInput,
    pub output_columns: TextInput,
    pub security_barrier: crate::db::catalog_mutation::ViewOption<bool>,
    pub security_invoker: crate::db::catalog_mutation::ViewOption<bool>,
    pub check_option: crate::db::catalog_mutation::ViewOption<String>,
    pub selected_field: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedViewDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub query: TextInput,
    pub tablespace: TextInput,
    pub with_data: bool,
    pub selected_field: usize,
    pub query_editable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub data_type: TextInput,
    pub increment: TextInput,
    pub min_value: crate::db::catalog_mutation::SequenceBound,
    pub max_value: crate::db::catalog_mutation::SequenceBound,
    pub start_value: TextInput,
    pub restart_value: TextInput,
    pub cache: TextInput,
    pub cycle: bool,
    pub owned_by: TextInput,
    pub selected_field: usize,
}

impl SequenceDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::SequenceDefinition) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            owner: definition.owner.clone().into(),
            comment: optional_text(&definition.comment),
            data_type: definition.data_type.clone().into(),
            increment: definition.increment.clone().into(),
            min_value: definition.min_value.clone(),
            max_value: definition.max_value.clone(),
            start_value: definition.start_value.clone().into(),
            restart_value: TextInput::default(),
            cache: definition.cache.clone().into(),
            cycle: definition.cycle,
            owned_by: definition
                .owned_by
                .as_ref()
                .map_or_else(TextInput::default, |(s, t, c)| {
                    format!("{s}.{t}.{c}").into()
                }),
            selected_field: 0,
        }
    }
    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            return Err(invalid("sequence name and schema are required"));
        }
        if self.data_type.value().trim().is_empty() {
            return Err(invalid("sequence data type is required"));
        }
        for (label, value) in [
            ("increment", &self.increment),
            ("start", &self.start_value),
            ("restart", &self.restart_value),
            ("cache", &self.cache),
        ] {
            if !value.value().trim().is_empty() && value.value().trim().parse::<i128>().is_err() {
                return Err(invalid(&format!("sequence {label} must be numeric")));
            }
        }
        for (label, bound) in [("minimum", &self.min_value), ("maximum", &self.max_value)] {
            if let crate::db::catalog_mutation::SequenceBound::Value(value) = bound
                && value.trim().parse::<i128>().is_err()
            {
                return Err(invalid(&format!("sequence {label} must be numeric")));
            }
        }
        if !self.owned_by.value().trim().is_empty()
            && self.owned_by.value().trim() != "NONE"
            && self.owned_by.value().split('.').count() != 3
        {
            return Err(invalid(
                "sequence OWNED BY must be schema.table.column or NONE",
            ));
        }
        Ok(())
    }
    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(10) as usize;
    }
    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.schema,
            2 => &mut self.owner,
            3 => &mut self.comment,
            4 => &mut self.data_type,
            5 => &mut self.increment,
            6 => &mut self.start_value,
            7 => &mut self.restart_value,
            8 => &mut self.cache,
            _ => &mut self.owned_by,
        }
    }
    pub fn insert(&mut self, c: char) {
        self.selected_input_mut().insert(c)
    }
    pub fn backspace(&mut self) {
        self.selected_input_mut().backspace()
    }
    pub fn delete(&mut self) {
        self.selected_input_mut().delete()
    }
    pub fn delete_previous_word(&mut self) {
        self.selected_input_mut().delete_previous_word()
    }
    pub fn delete_to_start(&mut self) {
        self.selected_input_mut().delete_to_start()
    }
    pub fn move_left(&mut self) {
        self.selected_input_mut().move_left()
    }
    pub fn move_right(&mut self) {
        self.selected_input_mut().move_right()
    }
    pub fn move_home(&mut self) {
        self.selected_input_mut().move_home()
    }
    pub fn move_end(&mut self) {
        self.selected_input_mut().move_end()
    }
}

impl MaterializedViewDraft {
    pub fn from_definition(
        definition: &crate::db::catalog_mutation::MaterializedViewDefinition,
    ) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            owner: definition.owner.clone().into(),
            comment: optional_text(&definition.comment),
            query: definition.query.clone().into(),
            tablespace: optional_text(&definition.tablespace),
            with_data: definition.populated,
            selected_field: 0,
            query_editable: false,
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            return Err(invalid("materialized view name and schema are required"));
        }
        if self.query_editable && self.query.value().trim().is_empty() {
            return Err(invalid("materialized view query is required"));
        }
        if self.query_editable
            && crate::sql::scan_statements(self.query.value(), crate::sql::SqlDialect::Postgres)
                .len()
                != 1
        {
            return Err(invalid(
                "materialized view definition must contain exactly one query",
            ));
        }
        Ok(())
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(6) as usize;
    }

    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.schema,
            2 => &mut self.owner,
            3 => &mut self.comment,
            4 => &mut self.query,
            _ => &mut self.tablespace,
        }
    }

    pub fn insert(&mut self, c: char) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().insert(c);
        }
    }
    pub fn backspace(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().backspace();
        }
    }
    pub fn delete(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().delete();
        }
    }
    pub fn delete_previous_word(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().delete_previous_word();
        }
    }
    pub fn delete_to_start(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().delete_to_start();
        }
    }
    pub fn move_left(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().move_left();
        }
    }
    pub fn move_right(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().move_right();
        }
    }
    pub fn move_home(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().move_home();
        }
    }
    pub fn move_end(&mut self) {
        if self.query_editable || self.selected_field != 4 {
            self.selected_input_mut().move_end();
        }
    }
}

impl ViewDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::ViewDefinition) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            owner: definition.owner.clone().into(),
            comment: optional_text(&definition.comment),
            query: definition.query.clone().into(),
            output_columns: definition.output_columns.join(", ").into(),
            security_barrier: definition.security_barrier.clone(),
            security_invoker: definition.security_invoker.clone(),
            check_option: definition.check_option.clone(),
            selected_field: 0,
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            return Err(invalid("view name and schema are required"));
        }
        if self.query.value().trim().is_empty() {
            return Err(invalid("view query is required"));
        }
        if crate::sql::scan_statements(self.query.value(), crate::sql::SqlDialect::Postgres).len()
            != 1
        {
            return Err(invalid("view definition must contain exactly one query"));
        }
        Ok(())
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(6) as usize;
    }
    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.schema,
            2 => &mut self.owner,
            3 => &mut self.comment,
            4 => &mut self.query,
            _ => &mut self.output_columns,
        }
    }
    pub fn insert(&mut self, c: char) {
        self.selected_input_mut().insert(c);
    }
    pub fn backspace(&mut self) {
        self.selected_input_mut().backspace();
    }
    pub fn delete(&mut self) {
        self.selected_input_mut().delete();
    }
    pub fn delete_previous_word(&mut self) {
        self.selected_input_mut().delete_previous_word();
    }
    pub fn delete_to_start(&mut self) {
        self.selected_input_mut().delete_to_start();
    }
    pub fn move_left(&mut self) {
        self.selected_input_mut().move_left();
    }
    pub fn move_right(&mut self) {
        self.selected_input_mut().move_right();
    }
    pub fn move_home(&mut self) {
        self.selected_input_mut().move_home();
    }
    pub fn move_end(&mut self) {
        self.selected_input_mut().move_end();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexColumnDraft {
    pub expression: TextInput,
    pub descending: bool,
    pub nulls_first: bool,
    pub is_expression: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub relation: TextInput,
    pub unique: bool,
    pub access_method: TextInput,
    pub columns: Vec<IndexColumnDraft>,
    pub include_columns: TextInput,
    pub predicate: TextInput,
    pub tablespace: TextInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstraintDraft {
    pub name: TextInput,
    pub database: TextInput,
    pub schema: TextInput,
    pub relation: TextInput,
    pub kind: crate::db::catalog_mutation::ConstraintDefinitionKind,
    pub columns: TextInput,
    pub referenced_schema: TextInput,
    pub referenced_relation: TextInput,
    pub referenced_columns: TextInput,
    pub match_type: TextInput,
    pub on_update: TextInput,
    pub on_delete: TextInput,
    pub expression: TextInput,
    pub no_inherit: bool,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub not_valid: bool,
    pub selected_field: usize,
}

impl ConstraintDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::ConstraintDefinition) -> Self {
        use crate::db::catalog_mutation::ConstraintDefinitionKind;
        let (
            columns,
            referenced_schema,
            referenced_relation,
            referenced_columns,
            match_type,
            on_update,
            on_delete,
            expression,
            no_inherit,
        ) = match &definition.kind {
            ConstraintDefinitionKind::PrimaryKey { columns }
            | ConstraintDefinitionKind::Unique { columns } => (
                columns.join(", "),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                false,
            ),
            ConstraintDefinitionKind::ForeignKey {
                columns,
                referenced_schema,
                referenced_relation,
                referenced_columns,
                match_type,
                on_update,
                on_delete,
            } => (
                columns.join(", "),
                referenced_schema.clone(),
                referenced_relation.clone(),
                referenced_columns.join(", "),
                match_type.clone(),
                on_update.clone(),
                on_delete.clone(),
                String::new(),
                false,
            ),
            ConstraintDefinitionKind::Check {
                expression,
                no_inherit,
            } => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                expression.clone(),
                *no_inherit,
            ),
        };
        Self {
            name: definition.name.clone().into(),
            database: definition.database.clone().into(),
            schema: definition.schema.clone().into(),
            relation: definition.relation.clone().into(),
            kind: definition.kind.clone(),
            columns: columns.into(),
            referenced_schema: referenced_schema.into(),
            referenced_relation: referenced_relation.into(),
            referenced_columns: referenced_columns.into(),
            match_type: match_type.into(),
            on_update: on_update.into(),
            on_delete: on_delete.into(),
            expression: expression.into(),
            no_inherit,
            deferrable: definition.deferrable,
            initially_deferred: definition.initially_deferred,
            not_valid: !definition.validated,
            selected_field: 0,
        }
    }

    pub fn new(
        kind: crate::db::catalog_mutation::ConstraintDefinitionKind,
        schema: &str,
        relation: &str,
    ) -> Self {
        let expression = match &kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { expression, .. } => {
                expression.clone()
            }
            _ => String::new(),
        };
        Self {
            name: TextInput::default(),
            database: TextInput::default(),
            schema: schema.into(),
            relation: relation.into(),
            kind,
            columns: TextInput::default(),
            referenced_schema: TextInput::default(),
            referenced_relation: TextInput::default(),
            referenced_columns: TextInput::default(),
            match_type: "SIMPLE".into(),
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
            expression: expression.into(),
            no_inherit: false,
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
            selected_field: 0,
        }
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = (self.selected_field as isize + delta).rem_euclid(10) as usize;
    }

    pub fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.selected_field {
            0 => Some(&mut self.name),
            1 => Some(&mut self.columns),
            2 => Some(&mut self.referenced_schema),
            3 => Some(&mut self.referenced_relation),
            4 => Some(&mut self.referenced_columns),
            5 => Some(&mut self.match_type),
            6 => Some(&mut self.on_update),
            7 => Some(&mut self.on_delete),
            8 => Some(&mut self.expression),
            9 => Some(&mut self.schema),
            _ => None,
        }
    }

    pub fn insert(&mut self, character: char) {
        if let Some(input) = self.selected_input_mut() {
            input.insert(character);
        }
    }
    pub fn backspace(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.backspace();
        }
    }
    pub fn delete(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete();
        }
    }
    pub fn delete_previous_word(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete_previous_word();
        }
    }
    pub fn delete_to_start(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete_to_start();
        }
    }
    pub fn move_left(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_left();
        }
    }
    pub fn move_right(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_right();
        }
    }
    pub fn move_home(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_home();
        }
    }
    pub fn move_end(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_end();
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        let columns = split_names(self.columns.value());
        if self.schema.value().trim().is_empty() || self.relation.value().trim().is_empty() {
            return Err(invalid("constraint relation is required"));
        }
        if self.initially_deferred && !self.deferrable {
            return Err(invalid(
                "initially deferred requires a deferrable constraint",
            ));
        }
        match &self.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                if self.expression.value().trim().is_empty() {
                    return Err(invalid("check expression is required"));
                }
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => {
                let refs = split_names(self.referenced_columns.value());
                if self.referenced_schema.value().trim().is_empty()
                    || self.referenced_relation.value().trim().is_empty()
                    || self.match_type.value().trim().is_empty()
                    || self.on_update.value().trim().is_empty()
                    || self.on_delete.value().trim().is_empty()
                {
                    return Err(invalid("foreign key target and actions are required"));
                }
                if columns.is_empty() || columns.len() != refs.len() {
                    return Err(invalid(
                        "foreign key source and referenced column counts must match",
                    ));
                }
                if columns.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(invalid(
                        "duplicate foreign key source columns are not allowed",
                    ));
                }
            }
            _ => {
                if columns.is_empty() {
                    return Err(invalid("constraint columns are required"));
                }
                if columns.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(invalid("duplicate constraint columns are not allowed"));
                }
            }
        }
        if self.not_valid
            && !matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    | crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. }
            )
        {
            return Err(invalid(
                "NOT VALID is supported only for foreign keys and checks",
            ));
        }
        Ok(())
    }
}

fn split_names(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}
fn invalid(reason: &str) -> crate::db::catalog_mutation::CatalogMutationError {
    crate::db::catalog_mutation::CatalogMutationError::InvalidDraft {
        reason: reason.into(),
    }
}

impl IndexDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::IndexDefinition) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            relation: definition.relation.clone().into(),
            unique: definition.unique,
            access_method: definition.access_method.clone().into(),
            columns: definition
                .columns
                .iter()
                .map(|column| IndexColumnDraft {
                    expression: column.expression.clone().into(),
                    descending: column.descending,
                    nulls_first: column.nulls_first,
                    is_expression: column.is_expression,
                })
                .collect(),
            include_columns: definition.include_columns.join(", ").into(),
            predicate: optional_text(&definition.predicate),
            tablespace: optional_text(&definition.tablespace),
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty()
            || self.schema.value().trim().is_empty()
            || self.relation.value().trim().is_empty()
            || self.access_method.value().trim().is_empty()
            || self.columns.is_empty()
            || self
                .columns
                .iter()
                .any(|column| column.expression.value().trim().is_empty())
        {
            return Err(
                crate::db::catalog_mutation::CatalogMutationError::InvalidDraft {
                    reason: "index name, relation, access method, and columns are required".into(),
                },
            );
        }
        Ok(())
    }
}

impl TableDraft {
    pub fn new(schema: impl Into<String>) -> Self {
        Self {
            name: TextInput::default(),
            schema: schema.into().into(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            columns: vec![ColumnDraft::new_added()],
            selected_section: CatalogEditorSection::General,
            selected_column: 0,
            selected_field: TableEditorField::Name,
            indexes: Vec::new(),
            constraints: Vec::new(),
        }
    }

    pub fn from_definition(definition: &crate::db::catalog_mutation::TableDefinition) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            owner: definition.owner.clone().into(),
            comment: optional_text(&definition.comment),
            columns: definition
                .columns
                .iter()
                .map(ColumnDraft::from_definition)
                .collect(),
            selected_section: CatalogEditorSection::General,
            selected_column: 0,
            selected_field: TableEditorField::Name,
            indexes: definition.indexes.clone(),
            constraints: definition.constraints.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            return Err(
                crate::db::catalog_mutation::CatalogMutationError::InvalidDraft {
                    reason: "table name and schema are required".into(),
                },
            );
        }
        let mut names = std::collections::HashSet::new();
        let mut column_count = 0;
        for (index, column) in self.columns.iter().enumerate() {
            if matches!(column.state, DraftRowState::Removed { .. }) {
                continue;
            }
            column_count += 1;
            if column.name.value().trim().is_empty() {
                return Err(invalid(&format!("column {} name is required", index + 1)));
            }
            if column.native_type.value().trim().is_empty() {
                return Err(invalid(&format!("column {} type is required", index + 1)));
            }
            if !names.insert(column.name.value().trim().to_owned()) {
                return Err(invalid("column names must be unique"));
            }
            if column.identity && !column.default_expression.value().trim().is_empty() {
                return Err(invalid(&format!(
                    "column {} identity cannot have a default",
                    index + 1
                )));
            }
        }
        if column_count == 0 {
            return Err(invalid("a table requires at least one column"));
        }
        Ok(())
    }

    pub fn validation_focus(&self) -> Option<(Option<usize>, TableEditorField, String)> {
        if self.name.value().trim().is_empty() {
            return Some((
                None,
                TableEditorField::Name,
                "table name is required".into(),
            ));
        }
        if self.schema.value().trim().is_empty() {
            return Some((
                None,
                TableEditorField::Schema,
                "table schema is required".into(),
            ));
        }
        let mut names = std::collections::HashSet::new();
        let mut count = 0;
        for (index, column) in self.columns.iter().enumerate() {
            if matches!(column.state, DraftRowState::Removed { .. }) {
                continue;
            }
            count += 1;
            if column.name.value().trim().is_empty() {
                return Some((
                    Some(index),
                    TableEditorField::ColumnName,
                    format!("column {} name is required", index + 1),
                ));
            }
            if column.native_type.value().trim().is_empty() {
                return Some((
                    Some(index),
                    TableEditorField::ColumnType,
                    format!("column {} type is required", index + 1),
                ));
            }
            if !names.insert(column.name.value().trim()) {
                return Some((
                    Some(index),
                    TableEditorField::ColumnName,
                    "column names must be unique".into(),
                ));
            }
            if column.identity && !column.default_expression.value().trim().is_empty() {
                return Some((
                    Some(index),
                    TableEditorField::ColumnIdentity,
                    format!("column {} identity cannot have a default", index + 1),
                ));
            }
        }
        (count == 0).then(|| {
            (
                None,
                TableEditorField::ColumnList,
                "a table requires at least one column".into(),
            )
        })
    }

    pub fn select_section(&mut self, delta: isize) {
        let current = self.selected_section as isize;
        self.selected_section = match (current + delta).clamp(0, 3) {
            0 => CatalogEditorSection::General,
            1 => CatalogEditorSection::Columns,
            2 => CatalogEditorSection::Indexes,
            _ => CatalogEditorSection::Constraints,
        };
    }

    pub fn move_column(&mut self, delta: isize) {
        let last = self.columns.len().saturating_sub(1) as isize;
        self.selected_column = (self.selected_column as isize + delta).clamp(0, last) as usize;
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field = self.selected_field.move_by(delta);
    }

    pub fn selected_column(&self) -> Option<&ColumnDraft> {
        self.columns.get(self.selected_column)
    }

    pub fn selected_column_mut(&mut self) -> Option<&mut ColumnDraft> {
        self.columns.get_mut(self.selected_column)
    }

    pub fn add_column(&mut self) {
        let mut column = ColumnDraft::new_added();
        column.ordinal_position = self.columns.len() as u32 + 1;
        self.columns.push(column);
        self.selected_column = self.columns.len() - 1;
        self.selected_field = TableEditorField::ColumnName;
    }

    pub fn remove_selected_column(&mut self) {
        if self.columns.len() == 1 {
            self.columns[0] = ColumnDraft::new_added();
            self.selected_column = 0;
            self.selected_field = TableEditorField::ColumnName;
            return;
        }
        if matches!(
            self.columns
                .get(self.selected_column)
                .map(|column| &column.state),
            Some(DraftRowState::Added)
        ) {
            self.columns.remove(self.selected_column);
        } else if let Some(column) = self.selected_column_mut()
            && let DraftRowState::Existing { id } = &column.state
        {
            column.state = DraftRowState::Removed { id: id.clone() };
        }
        self.selected_column = self.selected_column.min(self.columns.len() - 1);
        self.selected_field = TableEditorField::ColumnList;
    }

    pub fn toggle_selected_column_nullable(&mut self) {
        if let Some(column) = self.selected_column_mut() {
            column.nullable = !column.nullable;
        }
    }

    pub fn toggle_selected_column_identity(&mut self) {
        if let Some(column) = self.selected_column_mut() {
            column.identity = !column.identity;
            if column.identity {
                column.default_expression.set("");
            }
        }
    }

    fn selected_text_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.selected_field {
            TableEditorField::Name => Some(&mut self.name),
            TableEditorField::Schema => Some(&mut self.schema),
            TableEditorField::Owner => Some(&mut self.owner),
            TableEditorField::Comment => Some(&mut self.comment),
            TableEditorField::ColumnName => self
                .columns
                .get_mut(self.selected_column)
                .map(|column| &mut column.name),
            TableEditorField::ColumnType => self
                .columns
                .get_mut(self.selected_column)
                .map(|column| &mut column.native_type),
            TableEditorField::ColumnDefault => self
                .columns
                .get_mut(self.selected_column)
                .map(|column| &mut column.default_expression),
            TableEditorField::ColumnComment => self
                .columns
                .get_mut(self.selected_column)
                .map(|column| &mut column.comment),
            _ => None,
        }
    }

    pub fn insert(&mut self, character: char) {
        if let Some(input) = self.selected_text_input_mut() {
            input.insert(character);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.backspace();
        }
    }

    pub fn delete(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.delete();
        }
    }

    pub fn delete_previous_word(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.delete_previous_word();
        }
    }

    pub fn delete_to_start(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.delete_to_start();
        }
    }

    pub fn move_left(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.move_left();
        }
    }

    pub fn move_right(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.move_right();
        }
    }

    pub fn move_home(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.move_home();
        }
    }

    pub fn move_end(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.move_end();
        }
    }
}

impl ColumnDraft {
    pub fn new_added() -> Self {
        Self {
            row_id: Uuid::new_v4(),
            ordinal_position: 0,
            existing_name: None,
            name: TextInput::default(),
            native_type: "text".into(),
            nullable: true,
            default_expression: TextInput::default(),
            identity: false,
            generated_expression: TextInput::default(),
            collation: TextInput::default(),
            comment: TextInput::default(),
            state: DraftRowState::Added,
        }
    }

    fn from_definition(definition: &crate::db::catalog_mutation::ColumnDefinition) -> Self {
        Self {
            row_id: Uuid::new_v4(),
            ordinal_position: definition.ordinal_position,
            existing_name: Some(definition.name.clone()),
            name: definition.name.clone().into(),
            native_type: definition.native_type.clone().into(),
            nullable: definition.nullable,
            default_expression: optional_text(&definition.default_expression),
            identity: matches!(
                definition.identity,
                crate::db::catalog::OptionalMetadata::Supported(Some(true))
            ),
            generated_expression: optional_text(&definition.generated_expression),
            collation: optional_text(&definition.collation),
            comment: optional_text(&definition.comment),
            state: DraftRowState::Existing {
                id: CatalogId::new(
                    Uuid::nil(),
                    crate::db::catalog::CatalogKind::Column,
                    [definition.name.clone()],
                ),
            },
        }
    }
}

fn optional_text(value: &crate::db::catalog::OptionalMetadata<String>) -> TextInput {
    match value {
        crate::db::catalog::OptionalMetadata::Supported(Some(value)) => value.clone().into(),
        _ => TextInput::default(),
    }
}

impl SchemaDraft {
    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() {
            return Err(
                crate::db::catalog_mutation::CatalogMutationError::InvalidDraft {
                    reason: "schema name is required".into(),
                },
            );
        }
        if self.owner.value().trim().is_empty() {
            return Err(
                crate::db::catalog_mutation::CatalogMutationError::InvalidDraft {
                    reason: "schema owner is required".into(),
                },
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogDraft {
    Database(DatabaseDraft),
    Role(RoleDraft),
    Schema(SchemaDraft),
    Table(TableDraft),
    Index(IndexDraft),
    Constraint(ConstraintDraft),
    View(ViewDraft),
    MaterializedView(MaterializedViewDraft),
    Sequence(SequenceDraft),
}

impl CatalogDraft {
    pub fn move_field(&mut self, delta: isize) {
        match self {
            Self::Table(d) => d.move_field(delta),
            Self::Schema(d) => d.move_field(delta),
            Self::View(d) => d.move_field(delta),
            Self::MaterializedView(d) => d.move_field(delta),
            Self::Sequence(d) => d.move_field(delta),
            Self::Database(d) => d.move_field(delta),
            Self::Role(d) => d.move_field(delta),
            _ => {}
        }
    }
    pub fn insert(&mut self, c: char) {
        match self {
            Self::Table(d) => d.insert(c),
            Self::Schema(d) => d.insert(c),
            Self::View(d) => d.insert(c),
            Self::MaterializedView(d) => d.insert(c),
            Self::Sequence(d) => d.insert(c),
            Self::Database(d) => d.insert(c),
            Self::Role(d) => d.insert(c),
            _ => {}
        }
    }
    pub fn backspace(&mut self) {
        match self {
            Self::Table(d) => d.backspace(),
            Self::Schema(d) => d.backspace(),
            Self::View(d) => d.backspace(),
            Self::MaterializedView(d) => d.backspace(),
            Self::Sequence(d) => d.backspace(),
            Self::Database(d) => d.backspace(),
            Self::Role(d) => d.backspace(),
            _ => {}
        }
    }
    pub fn delete(&mut self) {
        match self {
            Self::Table(d) => d.delete(),
            Self::Schema(d) => d.delete(),
            Self::View(d) => d.delete(),
            Self::MaterializedView(d) => d.delete(),
            Self::Sequence(d) => d.delete(),
            Self::Database(d) => d.delete(),
            Self::Role(d) => d.delete(),
            _ => {}
        }
    }
    pub fn delete_previous_word(&mut self) {
        match self {
            Self::Table(d) => d.delete_previous_word(),
            Self::Schema(d) => d.delete_previous_word(),
            Self::View(d) => d.delete_previous_word(),
            Self::MaterializedView(d) => d.delete_previous_word(),
            Self::Sequence(d) => d.delete_previous_word(),
            _ => {}
        }
    }
    pub fn delete_to_start(&mut self) {
        match self {
            Self::Table(d) => d.delete_to_start(),
            Self::Schema(d) => d.delete_to_start(),
            Self::View(d) => d.delete_to_start(),
            Self::MaterializedView(d) => d.delete_to_start(),
            Self::Sequence(d) => d.delete_to_start(),
            _ => {}
        }
    }
    pub fn move_left(&mut self) {
        match self {
            Self::Table(d) => d.move_left(),
            Self::Schema(d) => d.move_left(),
            Self::View(d) => d.move_left(),
            Self::MaterializedView(d) => d.move_left(),
            Self::Sequence(d) => d.move_left(),
            _ => {}
        }
    }
    pub fn move_right(&mut self) {
        match self {
            Self::Table(d) => d.move_right(),
            Self::Schema(d) => d.move_right(),
            Self::View(d) => d.move_right(),
            Self::MaterializedView(d) => d.move_right(),
            Self::Sequence(d) => d.move_right(),
            _ => {}
        }
    }
    pub fn move_home(&mut self) {
        match self {
            Self::Table(d) => d.move_home(),
            Self::Schema(d) => d.move_home(),
            Self::View(d) => d.move_home(),
            Self::MaterializedView(d) => d.move_home(),
            Self::Sequence(d) => d.move_home(),
            _ => {}
        }
    }
    pub fn move_end(&mut self) {
        match self {
            Self::Table(d) => d.move_end(),
            Self::Schema(d) => d.move_end(),
            Self::View(d) => d.move_end(),
            Self::MaterializedView(d) => d.move_end(),
            Self::Sequence(d) => d.move_end(),
            _ => {}
        }
    }
}

pub use crate::db::catalog_mutation::{
    CatalogMutationExecutionMode, CatalogMutationPlan, CatalogObjectDefinition, ColumnDefinition,
    ConstraintDefinition, DatabaseDefinition, IndexColumnDefinition, IndexDefinition,
    MaterializedViewDefinition, RoleDefinition, SchemaDefinition, SequenceBound,
    SequenceDefinition, TableDefinition, ViewDefinition,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OwnerPickerState {
    pub open: bool,
    pub filter: TextInput,
    pub selected_name: Option<String>,
    pub message: Option<String>,
}

impl OwnerPickerState {
    pub fn open(&mut self, current_owner: &str, choices: &[CatalogOwnerChoice]) {
        self.open = true;
        self.filter.set("");
        self.selected_name = choices
            .iter()
            .find(|choice| choice.name == current_owner)
            .or_else(|| choices.first())
            .map(|choice| choice.name.clone());
        self.message = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.filter.set("");
        self.selected_name = None;
        self.message = None;
    }

    pub fn visible<'a>(&self, choices: &'a [CatalogOwnerChoice]) -> Vec<&'a CatalogOwnerChoice> {
        let filter = self.filter.value().to_lowercase();
        choices
            .iter()
            .filter(|choice| choice.name.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn reconcile(&mut self, choices: &[CatalogOwnerChoice]) {
        let visible = self.visible(choices);
        if visible
            .iter()
            .any(|choice| Some(choice.name.as_str()) == self.selected_name.as_deref())
        {
            return;
        }
        self.selected_name = visible.first().map(|choice| choice.name.clone());
    }

    pub fn insert_filter(&mut self, character: char, choices: &[CatalogOwnerChoice]) {
        self.filter.insert(character);
        self.reconcile(choices);
    }

    pub fn backspace_filter(&mut self, choices: &[CatalogOwnerChoice]) {
        self.filter.backspace();
        self.reconcile(choices);
    }

    pub fn delete_previous_word(&mut self, choices: &[CatalogOwnerChoice]) {
        self.filter.delete_previous_word();
        self.reconcile(choices);
    }

    pub fn delete_to_start(&mut self, choices: &[CatalogOwnerChoice]) {
        self.filter.delete_to_start();
        self.reconcile(choices);
    }

    pub fn move_selection(&mut self, delta: isize, choices: &[CatalogOwnerChoice]) {
        let visible = self.visible(choices);
        if visible.is_empty() {
            self.selected_name = None;
            return;
        }
        let current = visible
            .iter()
            .position(|choice| Some(choice.name.as_str()) == self.selected_name.as_deref())
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(visible.len() as isize) as usize;
        self.selected_name = Some(visible[next].name.clone());
    }

    pub fn selected<'a>(
        &self,
        choices: &'a [CatalogOwnerChoice],
    ) -> Option<&'a CatalogOwnerChoice> {
        self.visible(choices)
            .into_iter()
            .find(|choice| Some(choice.name.as_str()) == self.selected_name.as_deref())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEditorState {
    pub mode: CatalogMutationMode,
    pub anchor: CatalogMutationAnchor,
    pub object_type: Option<CatalogObjectType>,
    pub page: CatalogEditorPage,
    pub operation: Option<CatalogEditorOperation>,
    pub catalog_epoch: u64,
    pub options: Vec<CatalogMutationOption>,
    pub selected_option: usize,
    pub draft: Option<CatalogDraft>,
    pub baseline: Option<CatalogObjectDefinition>,
    pub plan: Option<CatalogMutationPlan>,
    pub error: Option<String>,
    pub owner_picker: OwnerPickerState,
}

impl CatalogEditorState {
    pub fn new(
        mode: CatalogMutationMode,
        anchor: CatalogMutationAnchor,
        catalog_epoch: u64,
        options: Vec<CatalogMutationOption>,
    ) -> Self {
        let object_type = matches!(mode, CatalogMutationMode::Edit).then(|| match &anchor {
            CatalogMutationAnchor::Catalog(id) => CatalogObjectType::Catalog(id.kind),
            _ => options.first().map(|option| option.object_type).unwrap_or(
                CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Schema),
            ),
        });
        Self {
            mode,
            anchor,
            object_type,
            page: if object_type.is_some() {
                CatalogEditorPage::Loading
            } else {
                CatalogEditorPage::ObjectPicker
            },
            operation: None,
            catalog_epoch,
            options,
            selected_option: 0,
            draft: None,
            baseline: None,
            plan: None,
            error: None,
            owner_picker: OwnerPickerState::default(),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.operation.is_some()
    }

    /// True when the schema form focuses the owner field, which the owner picker owns.
    pub fn owner_field_focused(&self) -> bool {
        matches!(
            self.draft.as_ref(),
            Some(CatalogDraft::Schema(draft)) if draft.selected_field == SCHEMA_OWNER_FIELD
        )
    }

    /// True when the owner role list is on screen, so it owns list keys instead of the form.
    pub fn owner_picker_active(&self) -> bool {
        self.owner_picker.open && self.owner_field_focused()
    }

    pub fn select_option(&mut self, selected: usize) -> bool {
        if self.is_busy() {
            return false;
        }
        let Some(object_type) = self.options.get(selected).map(|option| option.object_type) else {
            return false;
        };
        self.selected_option = selected;
        self.select_object_type(object_type)
    }

    pub fn select_object_type(&mut self, object_type: CatalogObjectType) -> bool {
        if self.is_busy() || self.mode != CatalogMutationMode::Create {
            return false;
        }
        self.object_type = Some(object_type);
        self.page = CatalogEditorPage::Form;
        if matches!(self.anchor, CatalogMutationAnchor::Profile { .. })
            && self.object_type
                == Some(CatalogObjectType::Catalog(
                    crate::db::catalog::CatalogKind::Database,
                ))
        {
            self.draft = Some(CatalogDraft::Database(DatabaseDraft::new("")));
        }
        if matches!(self.anchor, CatalogMutationAnchor::Profile { .. })
            && matches!(
                self.object_type,
                Some(CatalogObjectType::LoginRole | CatalogObjectType::Role)
            )
        {
            self.draft = Some(CatalogDraft::Role(RoleDraft::new(
                self.object_type == Some(CatalogObjectType::LoginRole),
            )));
        }
        if let (Some(id), Some(object_type)) = (
            match &self.anchor {
                CatalogMutationAnchor::Catalog(id) => Some(id),
                CatalogMutationAnchor::Group { schema: id, .. } => Some(id),
                CatalogMutationAnchor::Profile { .. } => None,
            },
            self.object_type,
        ) {
            if id.kind == crate::db::catalog::CatalogKind::Database
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Schema)
            {
                self.draft = Some(CatalogDraft::Schema(SchemaDraft::new()));
            }
            if id.kind == crate::db::catalog::CatalogKind::Schema
                && object_type == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Table)
            {
                self.draft = Some(CatalogDraft::Table(TableDraft::new(
                    id.native_path.get(1).cloned().unwrap_or_default(),
                )));
            }
            if id.kind == crate::db::catalog::CatalogKind::Schema
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Sequence)
            {
                self.draft = Some(CatalogDraft::Sequence(SequenceDraft {
                    name: TextInput::default(),
                    schema: id.native_path.get(1).cloned().unwrap_or_default().into(),
                    owner: TextInput::default(),
                    comment: TextInput::default(),
                    data_type: "bigint".into(),
                    increment: "1".into(),
                    min_value: crate::db::catalog_mutation::SequenceBound::Unset,
                    max_value: crate::db::catalog_mutation::SequenceBound::Unset,
                    start_value: "1".into(),
                    restart_value: TextInput::default(),
                    cache: "1".into(),
                    cycle: false,
                    owned_by: "NONE".into(),
                    selected_field: 0,
                }));
            }
            if id.kind == crate::db::catalog::CatalogKind::Database
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Database)
            {
                self.draft = Some(CatalogDraft::Database(DatabaseDraft::new(
                    id.native_path.first().cloned().unwrap_or_default(),
                )));
            }
            if id.kind == crate::db::catalog::CatalogKind::Table {
                let schema = id
                    .native_path
                    .get(1)
                    .map(String::as_str)
                    .unwrap_or_default();
                let relation = id
                    .native_path
                    .get(2)
                    .map(String::as_str)
                    .unwrap_or_default();
                let kind = match object_type {
                    CatalogObjectType::Catalog(kind) => kind,
                    _ => crate::db::catalog::CatalogKind::CheckConstraint,
                };
                let mut draft = ConstraintDraft::new(
                    match kind {
                        crate::db::catalog::CatalogKind::PrimaryKey => {
                            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey {
                                columns: Vec::new(),
                            }
                        }
                        crate::db::catalog::CatalogKind::UniqueConstraint => {
                            crate::db::catalog_mutation::ConstraintDefinitionKind::Unique {
                                columns: Vec::new(),
                            }
                        }
                        crate::db::catalog::CatalogKind::ForeignKey => {
                            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey {
                                columns: Vec::new(),
                                referenced_schema: String::new(),
                                referenced_relation: String::new(),
                                referenced_columns: Vec::new(),
                                match_type: "SIMPLE".into(),
                                on_update: "NO ACTION".into(),
                                on_delete: "NO ACTION".into(),
                            }
                        }
                        _ => crate::db::catalog_mutation::ConstraintDefinitionKind::Check {
                            expression: String::new(),
                            no_inherit: false,
                        },
                    },
                    schema,
                    relation,
                );
                draft.database = id
                    .native_path
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default()
                    .into();
                self.draft = Some(CatalogDraft::Constraint(draft));
            }
            if id.kind == crate::db::catalog::CatalogKind::Schema
                && object_type == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::View)
            {
                self.draft = Some(CatalogDraft::View(ViewDraft {
                    name: TextInput::default(),
                    schema: id.native_path.get(1).cloned().unwrap_or_default().into(),
                    owner: TextInput::default(),
                    comment: TextInput::default(),
                    query: TextInput::default(),
                    output_columns: TextInput::default(),
                    security_barrier: crate::db::catalog_mutation::ViewOption::unavailable(
                        "security_barrier capability is not loaded",
                    ),
                    security_invoker: crate::db::catalog_mutation::ViewOption::unavailable(
                        "security_invoker capability is not loaded",
                    ),
                    check_option: crate::db::catalog_mutation::ViewOption::unavailable(
                        "check option capability is not loaded",
                    ),
                    selected_field: 0,
                }));
            }
            if id.kind == crate::db::catalog::CatalogKind::Schema
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::MaterializedView)
            {
                self.draft = Some(CatalogDraft::MaterializedView(MaterializedViewDraft {
                    name: TextInput::default(),
                    schema: id.native_path.get(1).cloned().unwrap_or_default().into(),
                    owner: TextInput::default(),
                    comment: TextInput::default(),
                    query: TextInput::default(),
                    tablespace: TextInput::default(),
                    with_data: true,
                    selected_field: 0,
                    query_editable: true,
                }));
            }
        }
        self.error = None;
        true
    }

    pub fn begin_loading(&mut self, request_id: u64) -> bool {
        if self.is_busy() || self.object_type.is_none() || self.page != CatalogEditorPage::Loading {
            return false;
        }
        self.operation = Some(CatalogEditorOperation::LoadingDefinition { request_id });
        true
    }

    pub fn finish_loading(
        &mut self,
        request_id: u64,
        definition: Option<CatalogObjectDefinition>,
    ) -> bool {
        if self.operation != Some(CatalogEditorOperation::LoadingDefinition { request_id }) {
            return false;
        }
        self.operation = None;
        self.baseline = definition;
        if let Some(CatalogObjectDefinition::Database(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::Database(DatabaseDraft::from_definition(
                definition,
            )));
        }
        if let Some(CatalogObjectDefinition::Role(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::Role(RoleDraft::from_definition(definition)));
        }
        self.page = CatalogEditorPage::Form;
        true
    }

    pub fn accepts_definition_request(
        &self,
        request: &crate::db::catalog_mutation::CatalogObjectDefinitionRequest,
    ) -> bool {
        self.operation
            == Some(CatalogEditorOperation::LoadingDefinition {
                request_id: request.request_id,
            })
            && self.catalog_epoch == request.catalog_epoch
            && self.anchor == CatalogMutationAnchor::Catalog(request.object.clone())
    }

    pub fn begin_planning(&mut self, request_id: u64) -> bool {
        if self.page != CatalogEditorPage::Form || self.is_busy() || self.draft.is_none() {
            return false;
        }
        self.error = None;
        self.operation = Some(CatalogEditorOperation::Planning { request_id });
        true
    }

    pub fn planning_failed(&mut self, request_id: u64, message: impl Into<String>) -> bool {
        if self.operation != Some(CatalogEditorOperation::Planning { request_id }) {
            return false;
        }
        self.operation = None;
        self.error = Some(message.into());
        self.page = CatalogEditorPage::Form;
        true
    }

    pub fn plan_ready(&mut self, request_id: u64, plan: CatalogMutationPlan) -> bool {
        if self.operation != Some(CatalogEditorOperation::Planning { request_id })
            || plan.request.request_id != request_id
            || plan.request.catalog_epoch != self.catalog_epoch
        {
            return false;
        }
        self.operation = None;
        self.plan = Some(plan);
        self.page = CatalogEditorPage::SqlPreview;
        true
    }

    pub fn begin_apply(&mut self, request_id: u64) -> bool {
        if self.page != CatalogEditorPage::SqlPreview
            || self.operation.is_some()
            || self
                .plan
                .as_ref()
                .is_none_or(|plan| plan.request.request_id != request_id)
        {
            return false;
        }
        self.operation = Some(CatalogEditorOperation::Applying { request_id });
        true
    }

    pub fn apply_succeeded(
        &mut self,
        request_id: u64,
        connection: crate::identity::ConnectionIdentity,
        catalog_epoch: u64,
    ) -> bool {
        if self.operation != Some(CatalogEditorOperation::Applying { request_id })
            || catalog_epoch != self.catalog_epoch
            || self
                .plan
                .as_ref()
                .is_none_or(|plan| plan.request.connection != connection)
        {
            return false;
        }
        self.operation = None;
        true
    }

    pub fn cancel(&self) -> bool {
        true
    }

    pub fn set_validation_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.page = CatalogEditorPage::Form;
    }
}

impl Default for CatalogEditorState {
    fn default() -> Self {
        Self::new(
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Profile {
                profile_id: Uuid::nil(),
            },
            0,
            Vec::new(),
        )
    }
}
