use uuid::Uuid;

use crate::{
    db::{
        catalog::CatalogId,
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogObjectType, CatalogOwnerChoice,
        },
    },
    model::secret_text_input::SecretTextInput,
    model::text_input::TextInput,
    security::RedactedSecret,
};

#[derive(Clone, Eq, PartialEq)]
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
    password_input: SecretTextInput,
    pub valid_until: TextInput,
    pub memberships: TextInput,
    pub comment: TextInput,
    pub selected_field: usize,
    pub focus: CatalogFormFocus,
}

impl std::fmt::Debug for RoleDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoleDraft")
            .field("name", &self.name)
            .field("login", &self.login)
            .field("superuser", &self.superuser)
            .field("createdb", &self.createdb)
            .field("createrole", &self.createrole)
            .field("inherit", &self.inherit)
            .field("replication", &self.replication)
            .field("bypass_rls", &self.bypass_rls)
            .field("connection_limit", &self.connection_limit)
            .field("password_set", &self.password.is_some())
            .field("valid_until", &self.valid_until)
            .field("memberships", &self.memberships)
            .field("comment", &self.comment)
            .field("selected_field", &self.selected_field)
            .field("focus", &self.focus)
            .finish()
    }
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
            password_input: SecretTextInput::default(),
            valid_until: "infinity".into(),
            memberships: TextInput::default(),
            comment: TextInput::default(),
            selected_field: 0,
            focus: CatalogFormFocus::Name,
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
        let value = value.into();
        self.password_input.set(value.clone());
        self.password = (!value.is_empty()).then(|| RedactedSecret::new(value));
    }
    pub(crate) fn password_value(&self) -> &str {
        self.password_input.value()
    }
    pub(crate) fn password_insert(&mut self, character: char) {
        self.password_input.insert(character);
        self.sync_password();
    }
    pub(crate) fn password_paste(&mut self, text: &str) {
        self.password_input.paste(text);
        self.sync_password();
    }
    pub(crate) fn password_backspace(&mut self) {
        self.password_input.backspace();
        self.sync_password();
    }
    pub(crate) fn password_delete(&mut self) {
        self.password_input.delete();
        self.sync_password();
    }
    fn sync_password(&mut self) {
        self.password = (!self.password_input.value().is_empty())
            .then(|| RedactedSecret::new(self.password_input.value()));
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
        self.focus = move_catalog_form_focus(self.focus, delta, ROLE_FOCUS_ORDER, |_| true);
        self.selected_field = role_focus_index(self.focus);
    }
    pub(crate) fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            CatalogFormFocus::Name => Some(&mut self.name),
            CatalogFormFocus::ConnectionLimit => Some(&mut self.connection_limit),
            CatalogFormFocus::ValidUntil => Some(&mut self.valid_until),
            CatalogFormFocus::Memberships => Some(&mut self.memberships),
            CatalogFormFocus::Comment => Some(&mut self.comment),
            _ => None,
        }
    }

    pub fn toggle_focused(&mut self) {
        match self.focus {
            CatalogFormFocus::Login => self.login = !self.login,
            CatalogFormFocus::Superuser => self.superuser = !self.superuser,
            CatalogFormFocus::CreateDb => self.createdb = !self.createdb,
            CatalogFormFocus::CreateRole => self.createrole = !self.createrole,
            CatalogFormFocus::Inherit => self.inherit = !self.inherit,
            CatalogFormFocus::Replication => self.replication = !self.replication,
            CatalogFormFocus::BypassRls => self.bypass_rls = !self.bypass_rls,
            _ => {}
        }
    }

    pub fn insert(&mut self, c: char) {
        if self.focus == CatalogFormFocus::Password {
            self.password_insert(c);
        } else if let Some(i) = self.selected_input_mut() {
            i.insert(c)
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.focus == CatalogFormFocus::Password {
            self.password_paste(text);
        } else if let Some(input) = self.selected_input_mut() {
            input.paste(text);
        }
    }
    pub fn insert_password(&mut self, character: char) {
        if self.focus == CatalogFormFocus::Password {
            self.password_insert(character);
        }
    }
    pub fn backspace(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_backspace();
        } else if let Some(i) = self.selected_input_mut() {
            i.backspace()
        }
    }
    pub fn delete_previous_word(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_delete_previous_word();
        } else if let Some(input) = self.selected_input_mut() {
            input.delete_previous_word();
        }
    }
    pub fn delete_to_start(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_delete_to_start();
        } else if let Some(input) = self.selected_input_mut() {
            input.delete_to_start();
        }
    }
    pub fn delete(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_delete();
        } else if let Some(i) = self.selected_input_mut() {
            i.delete()
        }
    }
    pub(crate) fn password_delete_previous_word(&mut self) {
        self.password_input.delete_previous_word();
        self.sync_password();
    }
    pub(crate) fn password_delete_to_start(&mut self) {
        self.password_input.delete_to_start();
        self.sync_password();
    }
    pub(crate) fn password_move_left(&mut self) {
        self.password_input.move_left();
    }
    pub(crate) fn password_move_right(&mut self) {
        self.password_input.move_right();
    }
    pub(crate) fn password_move_home(&mut self) {
        self.password_input.move_home();
    }
    pub(crate) fn password_move_end(&mut self) {
        self.password_input.move_end();
    }
    pub(crate) fn password_undo(&mut self) {
        self.password_input.undo();
        self.sync_password();
    }
    pub(crate) fn password_redo(&mut self) {
        self.password_input.redo();
        self.sync_password();
    }
    pub(crate) fn password_finish_edit_group(&mut self) {
        self.password_input.finish_edit_group();
    }

    pub fn move_left(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_move_left();
        } else if let Some(input) = self.selected_input_mut() {
            input.move_left();
        }
    }
    pub fn move_right(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_move_right();
        } else if let Some(input) = self.selected_input_mut() {
            input.move_right();
        }
    }
    pub fn move_home(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_move_home();
        } else if let Some(input) = self.selected_input_mut() {
            input.move_home();
        }
    }
    pub fn move_end(&mut self) {
        if self.focus == CatalogFormFocus::Password {
            self.password_move_end();
        } else if let Some(input) = self.selected_input_mut() {
            input.move_end();
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
pub enum TableGeneralField {
    Name,
    Schema,
    Owner,
    Comment,
    ClickHouseEngine,
    ClickHouseOrderBy,
    ClickHousePartitionBy,
    ClickHouseTtl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableColumnField {
    Name,
    Type,
    Default,
    Comment,
    Nullable,
    Identity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableActionField {
    AddColumn,
    AddConstraintPrimaryKey,
    AddConstraintUnique,
    AddConstraintCheck,
    AddConstraintForeignKey,
    Constraint(usize),
    RemoveConstraint(usize),
    Review,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableEditorFocus {
    General(TableGeneralField),
    Columns,
    ColumnDetails(TableColumnField),
    Action(TableActionField),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogFormFocus {
    Name,
    Schema,
    Owner,
    Comment,
    OutputColumns,
    Query,
    Tablespace,
    WithData,
    DataType,
    Increment,
    MinValue,
    MaxValue,
    StartValue,
    RestartValue,
    Cache,
    Cycle,
    OwnedBy,
    SecurityBarrier,
    SecurityInvoker,
    CheckOption,
    Review,
    Cancel,
    Template,
    Encoding,
    LocaleProvider,
    Locale,
    Collation,
    Ctype,
    ConnectionLimit,
    AllowConnections,
    IsTemplate,
    Login,
    Password,
    ValidUntil,
    Memberships,
    Superuser,
    CreateDb,
    CreateRole,
    Inherit,
    Replication,
    BypassRls,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogFormFocusKind {
    Text,
    Choice,
    Toggle,
    Action,
    Disabled,
    ReadOnly,
}

const VIEW_FOCUS_ORDER: &[CatalogFormFocus] = &[
    CatalogFormFocus::Name,
    CatalogFormFocus::Schema,
    CatalogFormFocus::Owner,
    CatalogFormFocus::Comment,
    CatalogFormFocus::OutputColumns,
    CatalogFormFocus::Query,
    CatalogFormFocus::SecurityBarrier,
    CatalogFormFocus::SecurityInvoker,
    CatalogFormFocus::CheckOption,
    CatalogFormFocus::Review,
    CatalogFormFocus::Cancel,
];

const MATERIALIZED_VIEW_FOCUS_ORDER: &[CatalogFormFocus] = &[
    CatalogFormFocus::Name,
    CatalogFormFocus::Schema,
    CatalogFormFocus::Owner,
    CatalogFormFocus::Comment,
    CatalogFormFocus::Query,
    CatalogFormFocus::Tablespace,
    CatalogFormFocus::WithData,
    CatalogFormFocus::Review,
    CatalogFormFocus::Cancel,
];

const SEQUENCE_FOCUS_ORDER: &[CatalogFormFocus] = &[
    CatalogFormFocus::Name,
    CatalogFormFocus::Schema,
    CatalogFormFocus::Owner,
    CatalogFormFocus::Comment,
    CatalogFormFocus::DataType,
    CatalogFormFocus::Increment,
    CatalogFormFocus::MinValue,
    CatalogFormFocus::MaxValue,
    CatalogFormFocus::StartValue,
    CatalogFormFocus::RestartValue,
    CatalogFormFocus::Cache,
    CatalogFormFocus::Cycle,
    CatalogFormFocus::OwnedBy,
    CatalogFormFocus::Review,
    CatalogFormFocus::Cancel,
];

const DATABASE_FOCUS_ORDER: &[CatalogFormFocus] = &[
    CatalogFormFocus::Name,
    CatalogFormFocus::Owner,
    CatalogFormFocus::Comment,
    CatalogFormFocus::Template,
    CatalogFormFocus::Encoding,
    CatalogFormFocus::LocaleProvider,
    CatalogFormFocus::Locale,
    CatalogFormFocus::Collation,
    CatalogFormFocus::Ctype,
    CatalogFormFocus::Tablespace,
    CatalogFormFocus::ConnectionLimit,
    CatalogFormFocus::AllowConnections,
    CatalogFormFocus::IsTemplate,
    CatalogFormFocus::Review,
    CatalogFormFocus::Cancel,
];

const ROLE_FOCUS_ORDER: &[CatalogFormFocus] = &[
    CatalogFormFocus::Name,
    CatalogFormFocus::Comment,
    CatalogFormFocus::Login,
    CatalogFormFocus::Password,
    CatalogFormFocus::ValidUntil,
    CatalogFormFocus::ConnectionLimit,
    CatalogFormFocus::Superuser,
    CatalogFormFocus::CreateDb,
    CatalogFormFocus::CreateRole,
    CatalogFormFocus::Inherit,
    CatalogFormFocus::Replication,
    CatalogFormFocus::BypassRls,
    CatalogFormFocus::Memberships,
    CatalogFormFocus::Review,
    CatalogFormFocus::Cancel,
];

fn role_focus_index(focus: CatalogFormFocus) -> usize {
    ROLE_FOCUS_ORDER
        .iter()
        .position(|item| *item == focus)
        .unwrap_or(0)
}

fn database_input(draft: &DatabaseDraft, focus: CatalogFormFocus) -> Option<&TextInput> {
    if !draft.focus_enabled(focus) {
        return None;
    }
    match focus {
        CatalogFormFocus::Name => Some(&draft.name),
        CatalogFormFocus::Owner => Some(&draft.owner),
        CatalogFormFocus::Comment => Some(&draft.comment),
        CatalogFormFocus::Template => Some(&draft.template),
        CatalogFormFocus::Encoding => Some(&draft.encoding),
        CatalogFormFocus::LocaleProvider => Some(&draft.locale_provider),
        CatalogFormFocus::Locale => Some(&draft.locale),
        CatalogFormFocus::Collation => Some(&draft.collation),
        CatalogFormFocus::Ctype => Some(&draft.ctype),
        CatalogFormFocus::Tablespace => Some(&draft.tablespace),
        CatalogFormFocus::ConnectionLimit => Some(&draft.connection_limit),
        _ => None,
    }
}

fn database_input_mut(
    draft: &mut DatabaseDraft,
    focus: CatalogFormFocus,
) -> Option<&mut TextInput> {
    if !draft.focus_enabled(focus) {
        return None;
    }
    match focus {
        CatalogFormFocus::Name => Some(&mut draft.name),
        CatalogFormFocus::Owner => Some(&mut draft.owner),
        CatalogFormFocus::Comment => Some(&mut draft.comment),
        CatalogFormFocus::Template => Some(&mut draft.template),
        CatalogFormFocus::Encoding => Some(&mut draft.encoding),
        CatalogFormFocus::LocaleProvider => Some(&mut draft.locale_provider),
        CatalogFormFocus::Locale => Some(&mut draft.locale),
        CatalogFormFocus::Collation => Some(&mut draft.collation),
        CatalogFormFocus::Ctype => Some(&mut draft.ctype),
        CatalogFormFocus::Tablespace => Some(&mut draft.tablespace),
        CatalogFormFocus::ConnectionLimit => Some(&mut draft.connection_limit),
        _ => None,
    }
}

fn role_input(draft: &RoleDraft, focus: CatalogFormFocus) -> Option<&TextInput> {
    match focus {
        CatalogFormFocus::Name => Some(&draft.name),
        CatalogFormFocus::Comment => Some(&draft.comment),
        CatalogFormFocus::ValidUntil => Some(&draft.valid_until),
        CatalogFormFocus::ConnectionLimit => Some(&draft.connection_limit),
        CatalogFormFocus::Memberships => Some(&draft.memberships),
        _ => None,
    }
}

fn role_input_mut(draft: &mut RoleDraft, focus: CatalogFormFocus) -> Option<&mut TextInput> {
    match focus {
        CatalogFormFocus::Name => Some(&mut draft.name),
        CatalogFormFocus::Comment => Some(&mut draft.comment),
        CatalogFormFocus::ValidUntil => Some(&mut draft.valid_until),
        CatalogFormFocus::ConnectionLimit => Some(&mut draft.connection_limit),
        CatalogFormFocus::Memberships => Some(&mut draft.memberships),
        _ => None,
    }
}

fn move_catalog_form_focus(
    current: CatalogFormFocus,
    delta: isize,
    order: &[CatalogFormFocus],
    enabled: impl Fn(CatalogFormFocus) -> bool,
) -> CatalogFormFocus {
    let visible = order
        .iter()
        .copied()
        .filter(|focus| enabled(*focus))
        .collect::<Vec<_>>();
    let current = visible
        .iter()
        .position(|focus| *focus == current)
        .unwrap_or(0);
    visible[(current as isize + delta).rem_euclid(visible.len() as isize) as usize]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableColumnEditTarget {
    Existing { index: usize },
    New { insert_at: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableColumnEditSession {
    pub target: TableColumnEditTarget,
    pub draft: ColumnDraft,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableConstraintEditTarget {
    New,
    Existing { index: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableConstraintEditSession {
    pub target: TableConstraintEditTarget,
    pub draft: ConstraintDraft,
    pub return_focus: TableEditorFocus,
    pub error: Option<String>,
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

    pub(crate) fn selected_input_mut(&mut self) -> &mut TextInput {
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
    pub database_kind: crate::profile::DatabaseKind,
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
    pub focus: CatalogFormFocus,
    pub editable_creation_options: bool,
}

impl DatabaseDraft {
    pub fn from_definition(definition: &crate::db::catalog_mutation::DatabaseDefinition) -> Self {
        Self {
            database_kind: crate::profile::DatabaseKind::Postgres,
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
            focus: CatalogFormFocus::Name,
            editable_creation_options: false,
        }
    }
    pub fn new(_database: impl Into<String>) -> Self {
        Self {
            database_kind: crate::profile::DatabaseKind::Postgres,
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
            focus: CatalogFormFocus::Name,
            editable_creation_options: true,
        }
    }
    pub fn new_for_database(database_kind: crate::profile::DatabaseKind) -> Self {
        let mut draft = Self::new("");
        draft.database_kind = database_kind;
        draft
    }
    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.database_kind == crate::profile::DatabaseKind::ClickHouse {
            return if self.name.value().trim().is_empty() {
                Err(invalid("database name is required"))
            } else {
                Ok(())
            };
        }
        if matches!(
            self.database_kind,
            crate::profile::DatabaseKind::MySql | crate::profile::DatabaseKind::MariaDb
        ) {
            return if self.name.value().trim().is_empty() {
                Err(invalid("database name is required"))
            } else {
                Ok(())
            };
        }
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
        self.focus = move_catalog_form_focus(self.focus, delta, DATABASE_FOCUS_ORDER, |focus| {
            self.focus_enabled(focus)
        });
        self.selected_field = self.focus_index();
    }
    fn focus_index(&self) -> usize {
        DATABASE_FOCUS_ORDER
            .iter()
            .position(|item| *item == self.focus)
            .unwrap_or(0)
    }
    pub fn focus(&mut self, focus: CatalogFormFocus) -> bool {
        if !self.focus_enabled(focus) {
            return false;
        }
        self.focus = focus;
        self.selected_field = self.focus_index();
        true
    }
    pub fn focus_enabled(&self, focus: CatalogFormFocus) -> bool {
        if self.database_kind == crate::profile::DatabaseKind::ClickHouse {
            return matches!(focus, CatalogFormFocus::Name | CatalogFormFocus::Comment);
        }
        if matches!(
            self.database_kind,
            crate::profile::DatabaseKind::MySql | crate::profile::DatabaseKind::MariaDb
        ) {
            return matches!(focus, CatalogFormFocus::Name | CatalogFormFocus::Comment);
        }
        self.editable_creation_options
            || !matches!(
                focus,
                CatalogFormFocus::Owner
                    | CatalogFormFocus::Template
                    | CatalogFormFocus::Encoding
                    | CatalogFormFocus::LocaleProvider
                    | CatalogFormFocus::Locale
                    | CatalogFormFocus::Collation
                    | CatalogFormFocus::Ctype
                    | CatalogFormFocus::Tablespace
            )
    }
    fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        if self.selected_field != self.focus_index() {
            self.focus = match self.selected_field {
                0 => CatalogFormFocus::Name,
                1 => CatalogFormFocus::Owner,
                2 => CatalogFormFocus::Template,
                3 => CatalogFormFocus::Encoding,
                4 => CatalogFormFocus::LocaleProvider,
                5 => CatalogFormFocus::Locale,
                6 => CatalogFormFocus::Collation,
                7 => CatalogFormFocus::Ctype,
                8 => CatalogFormFocus::Tablespace,
                9 => CatalogFormFocus::ConnectionLimit,
                _ => CatalogFormFocus::Comment,
            };
        }
        if !self.focus_enabled(self.focus) {
            return None;
        }
        match self.focus {
            CatalogFormFocus::Name => Some(&mut self.name),
            CatalogFormFocus::Owner => Some(&mut self.owner),
            CatalogFormFocus::Template => Some(&mut self.template),
            CatalogFormFocus::Encoding => Some(&mut self.encoding),
            CatalogFormFocus::LocaleProvider => Some(&mut self.locale_provider),
            CatalogFormFocus::Locale => Some(&mut self.locale),
            CatalogFormFocus::Collation => Some(&mut self.collation),
            CatalogFormFocus::Ctype => Some(&mut self.ctype),
            CatalogFormFocus::Tablespace => Some(&mut self.tablespace),
            CatalogFormFocus::ConnectionLimit => Some(&mut self.connection_limit),
            CatalogFormFocus::Comment => Some(&mut self.comment),
            _ => None,
        }
    }
    pub fn insert(&mut self, c: char) {
        if let Some(input) = self.selected_input_mut() {
            input.insert(c);
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.selected_input_mut() {
            input.paste(text);
        }
    }
    pub fn toggle_focused(&mut self) {
        match self.focus {
            CatalogFormFocus::AllowConnections => self.allow_connections = !self.allow_connections,
            CatalogFormFocus::IsTemplate => self.is_template = !self.is_template,
            _ => {}
        }
    }
    pub fn backspace(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.backspace();
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
    pub fn delete(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete();
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
    pub database_kind: crate::profile::DatabaseKind,
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub columns: Vec<ColumnDraft>,
    pub selected_column: usize,
    pub focus: TableEditorFocus,
    pub column_editor: Option<TableColumnEditSession>,
    pub constraint_editor: Option<Box<TableConstraintEditSession>>,
    pub indexes: Vec<String>,
    pub constraints: Vec<String>,
    pub clickhouse_engine: TextInput,
    pub clickhouse_order_by: TextInput,
    pub clickhouse_partition_by: TextInput,
    pub clickhouse_ttl: TextInput,
    pub clickhouse_native_ddl: Option<String>,
    pub clickhouse_baseline_fingerprint: Option<String>,
    pub clickhouse_baseline_comment: Option<String>,
    pub clickhouse_baseline_columns:
        Option<Vec<crate::db::catalog_mutation::ClickHouseColumnDefinition>>,
    pub table_constraints: Vec<ConstraintDraft>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TableChangeSummary {
    pub properties_changed: bool,
    pub column_order_changed: bool,
    pub added_columns: usize,
    pub modified_columns: usize,
    pub removed_columns: usize,
}

impl TableChangeSummary {
    pub fn is_dirty(self) -> bool {
        self.properties_changed
            || self.column_order_changed
            || self.added_columns > 0
            || self.modified_columns > 0
            || self.removed_columns > 0
    }
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
    pub focus: CatalogFormFocus,
    pub clickhouse_native: Option<crate::db::catalog_mutation::ClickHouseViewDefinition>,
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
    pub focus: CatalogFormFocus,
    pub query_editable: bool,
    pub clickhouse_native:
        Option<crate::db::catalog_mutation::ClickHouseMaterializedViewDefinition>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceBoundKind {
    Default,
    NoLimit,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceBoundDraft {
    pub kind: SequenceBoundKind,
    pub value: TextInput,
}

impl SequenceBoundDraft {
    pub fn to_bound(&self) -> crate::db::catalog_mutation::SequenceBound {
        match self.kind {
            SequenceBoundKind::Default => crate::db::catalog_mutation::SequenceBound::Unset,
            SequenceBoundKind::NoLimit => crate::db::catalog_mutation::SequenceBound::NoLimit,
            SequenceBoundKind::Custom => {
                crate::db::catalog_mutation::SequenceBound::Value(self.value.value().to_owned())
            }
        }
    }

    fn cycle(&mut self, delta: isize) {
        let choices = [
            SequenceBoundKind::Default,
            SequenceBoundKind::NoLimit,
            SequenceBoundKind::Custom,
        ];
        let current = choices
            .iter()
            .position(|kind| *kind == self.kind)
            .unwrap_or(0);
        self.kind = choices[(current as isize + delta).rem_euclid(choices.len() as isize) as usize];
        if self.kind == SequenceBoundKind::Custom {
            self.value.set("");
        }
    }
}

impl From<crate::db::catalog_mutation::SequenceBound> for SequenceBoundDraft {
    fn from(bound: crate::db::catalog_mutation::SequenceBound) -> Self {
        match bound {
            crate::db::catalog_mutation::SequenceBound::Unset => Self {
                kind: SequenceBoundKind::Default,
                value: TextInput::default(),
            },
            crate::db::catalog_mutation::SequenceBound::NoLimit => Self {
                kind: SequenceBoundKind::NoLimit,
                value: TextInput::default(),
            },
            crate::db::catalog_mutation::SequenceBound::Value(value) => Self {
                kind: SequenceBoundKind::Custom,
                value: value.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceDraft {
    pub name: TextInput,
    pub schema: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub data_type: TextInput,
    pub increment: TextInput,
    pub min_value: SequenceBoundDraft,
    pub max_value: SequenceBoundDraft,
    pub start_value: TextInput,
    pub restart_value: TextInput,
    pub cache: TextInput,
    pub cycle: bool,
    pub owned_by: TextInput,
    pub focus: CatalogFormFocus,
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
            min_value: definition.min_value.clone().into(),
            max_value: definition.max_value.clone().into(),
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
            focus: CatalogFormFocus::Name,
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
            if bound.kind == SequenceBoundKind::Custom
                && bound.value.value().trim().parse::<i128>().is_err()
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

    pub fn validation_focus(&self) -> Option<CatalogFormFocus> {
        if self.name.value().trim().is_empty() {
            return Some(CatalogFormFocus::Name);
        }
        if self.schema.value().trim().is_empty() {
            return Some(CatalogFormFocus::Schema);
        }
        if self.data_type.value().trim().is_empty() {
            return Some(CatalogFormFocus::DataType);
        }
        for (focus, value) in [
            (CatalogFormFocus::Increment, &self.increment),
            (CatalogFormFocus::StartValue, &self.start_value),
            (CatalogFormFocus::RestartValue, &self.restart_value),
            (CatalogFormFocus::Cache, &self.cache),
        ] {
            if !value.value().trim().is_empty() && value.value().trim().parse::<i128>().is_err() {
                return Some(focus);
            }
        }
        for (focus, bound) in [
            (CatalogFormFocus::MinValue, &self.min_value),
            (CatalogFormFocus::MaxValue, &self.max_value),
        ] {
            if bound.kind == SequenceBoundKind::Custom
                && bound.value.value().trim().parse::<i128>().is_err()
            {
                return Some(focus);
            }
        }
        if !self.owned_by.value().trim().is_empty()
            && self.owned_by.value().trim() != "NONE"
            && self.owned_by.value().split('.').count() != 3
        {
            return Some(CatalogFormFocus::OwnedBy);
        }
        None
    }
    pub fn focus_enabled(&self, focus: CatalogFormFocus) -> bool {
        SEQUENCE_FOCUS_ORDER.contains(&focus)
    }

    pub fn focus(&mut self, focus: CatalogFormFocus) -> bool {
        if !self.focus_enabled(focus) {
            return false;
        }
        self.focus = focus;
        true
    }

    pub fn move_field(&mut self, delta: isize) {
        self.focus = move_catalog_form_focus(self.focus, delta, SEQUENCE_FOCUS_ORDER, |focus| {
            self.focus_enabled(focus)
        });
    }
    pub(crate) fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            CatalogFormFocus::Name => Some(&mut self.name),
            CatalogFormFocus::Schema => Some(&mut self.schema),
            CatalogFormFocus::Owner => Some(&mut self.owner),
            CatalogFormFocus::Comment => Some(&mut self.comment),
            CatalogFormFocus::DataType => Some(&mut self.data_type),
            CatalogFormFocus::Increment => Some(&mut self.increment),
            CatalogFormFocus::MinValue if self.min_value.kind == SequenceBoundKind::Custom => {
                Some(&mut self.min_value.value)
            }
            CatalogFormFocus::MaxValue if self.max_value.kind == SequenceBoundKind::Custom => {
                Some(&mut self.max_value.value)
            }
            CatalogFormFocus::StartValue => Some(&mut self.start_value),
            CatalogFormFocus::RestartValue => Some(&mut self.restart_value),
            CatalogFormFocus::Cache => Some(&mut self.cache),
            CatalogFormFocus::OwnedBy => Some(&mut self.owned_by),
            _ => None,
        }
    }
    pub fn insert(&mut self, c: char) {
        if let Some(input) = self.selected_input_mut() {
            input.insert(c)
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.selected_input_mut() {
            input.paste(text);
        }
    }
    pub fn backspace(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.backspace()
        }
    }
    pub fn delete(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete()
        }
    }
    pub fn delete_previous_word(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete_previous_word()
        }
    }
    pub fn delete_to_start(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.delete_to_start()
        }
    }
    pub fn move_left(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_left()
        }
    }
    pub fn move_right(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_right()
        }
    }
    pub fn move_home(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_home()
        }
    }
    pub fn move_end(&mut self) {
        if let Some(input) = self.selected_input_mut() {
            input.move_end()
        }
    }

    pub fn cycle_focused_choice(&mut self, delta: isize) -> bool {
        let bound = match self.focus {
            CatalogFormFocus::MinValue => &mut self.min_value,
            CatalogFormFocus::MaxValue => &mut self.max_value,
            _ => return false,
        };
        bound.cycle(delta);
        true
    }

    pub fn toggle_focused(&mut self) -> bool {
        if self.focus != CatalogFormFocus::Cycle {
            return false;
        }
        self.cycle = !self.cycle;
        true
    }
}

impl MaterializedViewDraft {
    pub fn from_clickhouse_native(
        definition: &crate::db::catalog_mutation::ClickHouseMaterializedViewDefinition,
    ) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.database.clone().into(),
            owner: TextInput::default(),
            comment: optional_text(&definition.comment),
            query: definition.query.clone().into(),
            tablespace: TextInput::default(),
            with_data: false,
            focus: CatalogFormFocus::Name,
            query_editable: false,
            clickhouse_native: Some(definition.clone()),
        }
    }

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
            focus: CatalogFormFocus::Name,
            query_editable: false,
            clickhouse_native: definition.clickhouse_native.clone(),
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

    pub fn validation_focus(&self) -> Option<CatalogFormFocus> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            Some(if self.name.value().trim().is_empty() {
                CatalogFormFocus::Name
            } else {
                CatalogFormFocus::Schema
            })
        } else if self.query_editable && self.query.value().trim().is_empty() {
            Some(CatalogFormFocus::Query)
        } else {
            None
        }
    }

    pub fn focus_enabled(&self, focus: CatalogFormFocus, allow_with_data: bool) -> bool {
        MATERIALIZED_VIEW_FOCUS_ORDER.contains(&focus)
            && (focus != CatalogFormFocus::Query || self.query_editable)
            && (focus != CatalogFormFocus::WithData || allow_with_data)
    }

    pub fn focus(&mut self, focus: CatalogFormFocus, allow_with_data: bool) -> bool {
        if !self.focus_enabled(focus, allow_with_data) {
            return false;
        }
        self.focus = focus;
        true
    }

    pub fn move_field(&mut self, delta: isize, allow_with_data: bool) {
        self.focus =
            move_catalog_form_focus(self.focus, delta, MATERIALIZED_VIEW_FOCUS_ORDER, |focus| {
                self.focus_enabled(focus, allow_with_data)
            });
    }

    pub(crate) fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            CatalogFormFocus::Name => Some(&mut self.name),
            CatalogFormFocus::Schema => Some(&mut self.schema),
            CatalogFormFocus::Owner => Some(&mut self.owner),
            CatalogFormFocus::Comment => Some(&mut self.comment),
            CatalogFormFocus::Query if self.query_editable => Some(&mut self.query),
            CatalogFormFocus::Tablespace => Some(&mut self.tablespace),
            _ => None,
        }
    }

    pub fn insert(&mut self, c: char) {
        if let Some(input) = self.selected_input_mut() {
            input.insert(c);
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.selected_input_mut() {
            input.paste(text);
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

    pub fn toggle_focused(&mut self, create_mode: bool) -> bool {
        if !create_mode || self.focus != CatalogFormFocus::WithData {
            return false;
        }
        self.with_data = !self.with_data;
        true
    }
}

impl ViewDraft {
    pub fn from_clickhouse_native(
        definition: &crate::db::catalog_mutation::ClickHouseViewDefinition,
    ) -> Self {
        Self {
            name: definition.name.clone().into(),
            schema: definition.database.clone().into(),
            owner: TextInput::default(),
            comment: optional_text(&definition.comment),
            query: definition.query.clone().into(),
            output_columns: TextInput::default(),
            security_barrier: crate::db::catalog_mutation::ViewOption::unavailable(
                "not applicable to ClickHouse",
            ),
            security_invoker: crate::db::catalog_mutation::ViewOption::unavailable(
                "not applicable to ClickHouse",
            ),
            check_option: crate::db::catalog_mutation::ViewOption::unavailable(
                "not applicable to ClickHouse",
            ),
            focus: CatalogFormFocus::Name,
            clickhouse_native: Some(definition.clone()),
        }
    }

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
            focus: CatalogFormFocus::Name,
            clickhouse_native: definition.clickhouse_native.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), crate::db::catalog_mutation::CatalogMutationError> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            return Err(invalid("view name and schema are required"));
        }
        if self.query.value().trim().is_empty() {
            return Err(invalid("view query is required"));
        }
        if (self.clickhouse_native.is_some() && !safe_view_query(self.query.value()))
            || (self.clickhouse_native.is_none()
                && crate::sql::scan_statements(
                    self.query.value(),
                    crate::sql::SqlDialect::Postgres,
                )
                .len()
                    != 1)
        {
            return Err(invalid("view definition must contain exactly one query"));
        }
        Ok(())
    }

    pub fn validation_focus(&self) -> Option<CatalogFormFocus> {
        if self.name.value().trim().is_empty() || self.schema.value().trim().is_empty() {
            Some(if self.name.value().trim().is_empty() {
                CatalogFormFocus::Name
            } else {
                CatalogFormFocus::Schema
            })
        } else if self.query.value().trim().is_empty()
            || crate::sql::scan_statements(self.query.value(), crate::sql::SqlDialect::Postgres)
                .len()
                != 1
        {
            Some(CatalogFormFocus::Query)
        } else {
            None
        }
    }

    pub fn focus_enabled(&self, focus: CatalogFormFocus) -> bool {
        VIEW_FOCUS_ORDER.contains(&focus)
            && match focus {
                CatalogFormFocus::SecurityBarrier => {
                    self.security_barrier.availability.is_available()
                }
                CatalogFormFocus::SecurityInvoker => {
                    self.security_invoker.availability.is_available()
                }
                CatalogFormFocus::CheckOption => self.check_option.availability.is_available(),
                _ => true,
            }
    }

    pub fn focus(&mut self, focus: CatalogFormFocus) -> bool {
        if !self.focus_enabled(focus) {
            return false;
        }
        self.focus = focus;
        true
    }

    pub fn move_field(&mut self, delta: isize) {
        self.focus = move_catalog_form_focus(self.focus, delta, VIEW_FOCUS_ORDER, |focus| {
            self.focus_enabled(focus)
        });
    }
    pub(crate) fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            CatalogFormFocus::Name => Some(&mut self.name),
            CatalogFormFocus::Schema => Some(&mut self.schema),
            CatalogFormFocus::Owner => Some(&mut self.owner),
            CatalogFormFocus::Comment => Some(&mut self.comment),
            CatalogFormFocus::Query => Some(&mut self.query),
            CatalogFormFocus::OutputColumns => Some(&mut self.output_columns),
            _ => None,
        }
    }
    pub fn insert(&mut self, c: char) {
        if let Some(input) = self.selected_input_mut() {
            input.insert(c);
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.selected_input_mut() {
            input.paste(text);
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

    pub fn cycle_focused_choice(&mut self, delta: isize) -> bool {
        fn cycle<T: Clone + PartialEq>(value: &mut Option<T>, choices: &[Option<T>], delta: isize) {
            let current = choices
                .iter()
                .position(|choice| choice == value)
                .unwrap_or(0);
            *value = choices
                [(current as isize + delta).rem_euclid(choices.len() as isize) as usize]
                .clone();
        }

        match self.focus {
            CatalogFormFocus::SecurityBarrier
                if self.security_barrier.availability.is_available() =>
            {
                cycle(
                    &mut self.security_barrier.value,
                    &[None, Some(true), Some(false)],
                    delta,
                );
            }
            CatalogFormFocus::SecurityInvoker
                if self.security_invoker.availability.is_available() =>
            {
                cycle(
                    &mut self.security_invoker.value,
                    &[None, Some(true), Some(false)],
                    delta,
                );
            }
            CatalogFormFocus::CheckOption if self.check_option.availability.is_available() => {
                cycle(
                    &mut self.check_option.value,
                    &[None, Some("LOCAL".into()), Some("CASCADED".into())],
                    delta,
                );
            }
            _ => return false,
        }
        true
    }
}

fn safe_view_query(query: &str) -> bool {
    let query = query.trim();
    !query.is_empty()
        && !query.contains(';')
        && !query.contains("--")
        && !query.contains("/*")
        && !query.contains("*/")
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
        let field_count = self.field_count();
        self.selected_field =
            (self.selected_field as isize + delta).rem_euclid(field_count as isize) as usize;
    }

    pub fn field_count(&self) -> usize {
        match self.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => 2,
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => 7,
        }
    }

    pub fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        self.input_for_field_mut(self.selected_field)
    }

    pub(crate) fn input_for_field(&self, field: usize) -> Option<&TextInput> {
        match field {
            0 => Some(&self.name),
            1 => match self.kind {
                crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                    Some(&self.expression)
                }
                _ => Some(&self.columns),
            },
            2 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&self.referenced_schema)
            }
            3 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&self.referenced_relation)
            }
            4 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&self.referenced_columns)
            }
            5 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&self.on_update)
            }
            6 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&self.on_delete)
            }
            _ => None,
        }
    }

    pub(crate) fn input_for_field_mut(&mut self, field: usize) -> Option<&mut TextInput> {
        match field {
            0 => Some(&mut self.name),
            1 => match self.kind {
                crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                    Some(&mut self.expression)
                }
                _ => Some(&mut self.columns),
            },
            2 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&mut self.referenced_schema)
            }
            3 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&mut self.referenced_relation)
            }
            4 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&mut self.referenced_columns)
            }
            5 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&mut self.on_update)
            }
            6 if matches!(
                self.kind,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
            ) =>
            {
                Some(&mut self.on_delete)
            }
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

    pub fn validation_focus(&self) -> Option<(usize, String)> {
        let columns = split_names(self.columns.value());
        match &self.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => self
                .expression
                .value()
                .trim()
                .is_empty()
                .then(|| (1, "check expression is required".into())),
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => {
                if columns.is_empty() {
                    return Some((1, "foreign key columns are required".into()));
                }
                if columns.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Some((1, "duplicate foreign key columns are not allowed".into()));
                }
                if self.referenced_schema.value().trim().is_empty() {
                    return Some((2, "target schema is required".into()));
                }
                if self.referenced_relation.value().trim().is_empty() {
                    return Some((3, "target table is required".into()));
                }
                let referenced = split_names(self.referenced_columns.value());
                if referenced.is_empty() {
                    return Some((4, "target columns are required".into()));
                }
                if columns.len() != referenced.len() {
                    return Some((
                        4,
                        "foreign key source and target column counts must match".into(),
                    ));
                }
                if self.on_update.value().trim().is_empty() {
                    return Some((5, "on update action is required".into()));
                }
                if self.on_delete.value().trim().is_empty() {
                    return Some((6, "on delete action is required".into()));
                }
                None
            }
            _ => {
                if columns.is_empty() {
                    return Some((1, "constraint columns are required".into()));
                }
                columns
                    .windows(2)
                    .any(|pair| pair[0] == pair[1])
                    .then(|| (1, "duplicate constraint columns are not allowed".into()))
            }
        }
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
    pub fn supports_structured_table_constraints(&self) -> bool {
        matches!(
            self.database_kind,
            crate::profile::DatabaseKind::Sqlite | crate::profile::DatabaseKind::DuckDb
        )
    }

    pub fn begin_new_table_constraint(
        &mut self,
        kind: crate::db::catalog_mutation::ConstraintDefinitionKind,
    ) -> Result<(), String> {
        if self.column_editor.is_some() || self.constraint_editor.is_some() {
            return Err("finish the open details editor first".into());
        }
        if !self.supports_structured_table_constraints() {
            return Err(
                "structured table constraints are available for SQLite and DuckDB tables only"
                    .into(),
            );
        }
        let schema = self.schema.value().to_owned();
        let relation = self.name.value().to_owned();
        let mut constraint = ConstraintDraft::new(kind, &schema, &relation);
        constraint.name = self.next_constraint_name(&relation).into();
        match &constraint.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => {
                constraint.columns = self
                    .columns
                    .iter()
                    .filter(|column| {
                        !matches!(column.state, DraftRowState::Removed { .. })
                            && !column.name.value().trim().is_empty()
                    })
                    .map(|column| column.name.value().trim().to_owned())
                    .next()
                    .unwrap_or_default()
                    .into();
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {}
        }
        if matches!(
            &constraint.kind,
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
        ) {
            constraint.referenced_schema = schema.into();
        }
        let return_focus = match &constraint.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. } => {
                TableActionField::AddConstraintPrimaryKey
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. } => {
                TableActionField::AddConstraintUnique
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                TableActionField::AddConstraintCheck
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => {
                TableActionField::AddConstraintForeignKey
            }
        };
        self.constraint_editor = Some(Box::new(TableConstraintEditSession {
            target: TableConstraintEditTarget::New,
            draft: constraint,
            return_focus: TableEditorFocus::Action(return_focus),
            error: None,
        }));
        Ok(())
    }

    pub fn begin_edit_table_constraint(&mut self, index: usize) -> bool {
        if self.column_editor.is_some() || self.constraint_editor.is_some() {
            return false;
        }
        let Some(constraint) = self.table_constraints.get(index).cloned() else {
            return false;
        };
        self.focus = TableEditorFocus::Action(TableActionField::Constraint(index));
        self.constraint_editor = Some(Box::new(TableConstraintEditSession {
            target: TableConstraintEditTarget::Existing { index },
            draft: constraint,
            return_focus: TableEditorFocus::Action(TableActionField::Constraint(index)),
            error: None,
        }));
        true
    }

    pub fn validate_table_constraint_details(&self) -> Option<(usize, String)> {
        self.constraint_editor
            .as_ref()
            .and_then(|session| session.draft.validation_focus())
    }

    pub fn confirm_table_constraint_details(&mut self) -> bool {
        let Some(session) = self.constraint_editor.take() else {
            return false;
        };
        match session.target {
            TableConstraintEditTarget::Existing { index } => {
                let Some(constraint) = self.table_constraints.get_mut(index) else {
                    self.focus = TableEditorFocus::Action(TableActionField::AddColumn);
                    return false;
                };
                *constraint = session.draft;
                self.focus = TableEditorFocus::Action(TableActionField::Constraint(index));
            }
            TableConstraintEditTarget::New => {
                let index = self.table_constraints.len();
                self.table_constraints.push(session.draft);
                self.focus = TableEditorFocus::Action(TableActionField::Constraint(index));
            }
        }
        true
    }

    pub fn cancel_table_constraint_details(&mut self) {
        let Some(session) = self.constraint_editor.take() else {
            return;
        };
        self.focus = session.return_focus;
    }

    pub fn move_table_constraint_details_field(&mut self, delta: isize) {
        if let Some(session) = self.constraint_editor.as_mut() {
            session.draft.move_field(delta);
        }
    }

    fn next_constraint_name(&self, relation: &str) -> String {
        let stem = if relation.trim().is_empty() {
            "table"
        } else {
            relation.trim()
        };
        let mut suffix = self.table_constraints.len() + 1;
        loop {
            let candidate = format!("{stem}_constraint_{suffix}");
            if !self
                .table_constraints
                .iter()
                .any(|constraint| constraint.name.value() == candidate)
            {
                return candidate;
            }
            suffix += 1;
        }
    }

    pub fn move_selected_constraint_field(&mut self, delta: isize) {
        if let TableEditorFocus::Action(TableActionField::Constraint(index)) = self.focus
            && let Some(constraint) = self.table_constraints.get_mut(index)
        {
            let field_count = match constraint.kind {
                crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
                | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. }
                | crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => 2,
                crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => 7,
            };
            constraint.selected_field =
                (constraint.selected_field as isize + delta).rem_euclid(field_count) as usize;
        }
    }
    pub fn add_constraint(&mut self, draft: ConstraintDraft) {
        let index = self.table_constraints.len();
        self.table_constraints.push(draft);
        self.focus = TableEditorFocus::Action(TableActionField::Constraint(index));
    }

    pub fn remove_constraint(&mut self, index: usize) -> bool {
        if self.constraint_editor.is_some() || self.column_editor.is_some() {
            return false;
        }
        if index < self.table_constraints.len() {
            self.table_constraints.remove(index);
            if self.table_constraints.is_empty() {
                self.focus = TableEditorFocus::Action(TableActionField::AddColumn);
            } else if let TableEditorFocus::Action(
                TableActionField::Constraint(selected)
                | TableActionField::RemoveConstraint(selected),
            ) = self.focus
            {
                self.focus = TableEditorFocus::Action(TableActionField::Constraint(
                    selected.min(self.table_constraints.len() - 1),
                ));
            }
            true
        } else {
            false
        }
    }

    pub fn new(schema: impl Into<String>) -> Self {
        Self::new_for_database(schema, crate::profile::DatabaseKind::Postgres)
    }

    pub fn new_for_database(
        schema: impl Into<String>,
        database_kind: crate::profile::DatabaseKind,
    ) -> Self {
        Self {
            database_kind,
            name: TextInput::default(),
            schema: schema.into().into(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            columns: vec![ColumnDraft::new_added_for_database(database_kind)],
            selected_column: 0,
            focus: TableEditorFocus::General(TableGeneralField::Name),
            column_editor: None,
            constraint_editor: None,
            indexes: Vec::new(),
            constraints: Vec::new(),
            clickhouse_engine: "MergeTree".into(),
            clickhouse_order_by: TextInput::default(),
            clickhouse_partition_by: TextInput::default(),
            clickhouse_ttl: TextInput::default(),
            clickhouse_native_ddl: None,
            clickhouse_baseline_fingerprint: None,
            clickhouse_baseline_comment: None,
            clickhouse_baseline_columns: None,
            table_constraints: Vec::new(),
        }
    }

    pub fn from_definition(definition: &crate::db::catalog_mutation::TableDefinition) -> Self {
        Self::from_definition_for_database(definition, crate::profile::DatabaseKind::Postgres)
    }

    pub fn from_definition_for_database(
        definition: &crate::db::catalog_mutation::TableDefinition,
        database_kind: crate::profile::DatabaseKind,
    ) -> Self {
        Self {
            database_kind,
            name: definition.name.clone().into(),
            schema: definition.schema.clone().into(),
            owner: definition.owner.clone().into(),
            comment: optional_text(&definition.comment),
            columns: definition
                .columns
                .iter()
                .map(ColumnDraft::from_definition)
                .collect(),
            selected_column: 0,
            focus: TableEditorFocus::General(TableGeneralField::Name),
            column_editor: None,
            constraint_editor: None,
            indexes: definition.indexes.clone(),
            constraints: definition.constraints.clone(),
            clickhouse_engine: "MergeTree".into(),
            clickhouse_order_by: TextInput::default(),
            clickhouse_partition_by: TextInput::default(),
            clickhouse_ttl: TextInput::default(),
            clickhouse_native_ddl: None,
            clickhouse_baseline_fingerprint: None,
            clickhouse_baseline_comment: None,
            clickhouse_baseline_columns: None,
            table_constraints: definition
                .structured_constraints
                .iter()
                .map(ConstraintDraft::from_definition)
                .collect(),
        }
    }

    pub fn from_clickhouse_definition(
        definition: &crate::db::catalog_mutation::ClickHouseTableDefinition,
    ) -> Self {
        let mut draft = Self::new_for_database(
            &definition.database,
            crate::profile::DatabaseKind::ClickHouse,
        );
        draft.name = definition.name.clone().into();
        draft.comment = optional_text(&definition.comment);
        draft.clickhouse_engine = definition.engine.clone().into();
        if let Some(order_by) = clickhouse_order_by_clause(&definition.canonical_ddl) {
            draft.clickhouse_order_by = order_by.into();
        }
        draft.clickhouse_native_ddl = Some(definition.canonical_ddl.clone());
        draft.clickhouse_baseline_fingerprint = Some(definition.baseline_fingerprint.clone());
        draft.clickhouse_baseline_columns = Some(definition.columns.clone());
        draft.clickhouse_baseline_comment = match &definition.comment {
            crate::db::catalog::OptionalMetadata::Supported(value) => {
                Some(value.clone().unwrap_or_default())
            }
            _ => None,
        };
        draft.columns = definition
            .columns
            .iter()
            .map(|column| ColumnDraft {
                row_id: Uuid::new_v4(),
                ordinal_position: column.ordinal_position,
                existing_name: Some(column.name.clone()),
                name: column.name.clone().into(),
                native_type: column.native_type.clone().into(),
                nullable: column.native_type.starts_with("Nullable("),
                default_expression: column.default_expression.clone().into(),
                identity: false,
                generated_expression: if matches!(
                    column.default_kind.as_str(),
                    "MATERIALIZED" | "ALIAS"
                ) {
                    column.default_expression.clone().into()
                } else {
                    TextInput::default()
                },
                collation: TextInput::default(),
                comment: optional_text(&column.comment),
                state: DraftRowState::Existing {
                    id: CatalogId::new(
                        Uuid::nil(),
                        crate::db::catalog::CatalogKind::Column,
                        [
                            definition.database.clone(),
                            definition.database.clone(),
                            definition.name.clone(),
                            column.name.clone(),
                        ],
                    ),
                },
            })
            .collect();
        draft
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
        if self.database_kind != crate::profile::DatabaseKind::ClickHouse && column_count == 0 {
            return Err(invalid("a table requires at least one column"));
        }
        if !self.table_constraints.is_empty() && !self.supports_structured_table_constraints() {
            return Err(invalid(
                "structured table constraints are supported for SQLite and DuckDB table creation only",
            ));
        }
        for constraint in &self.table_constraints {
            constraint.validate()?;
        }
        Ok(())
    }

    pub fn create_change_summary(&self) -> TableChangeSummary {
        let properties_changed = !self.name.value().is_empty()
            || self.owner.value() != ""
            || !self.comment.value().is_empty()
            || !self.indexes.is_empty()
            || !self.constraints.is_empty()
            || !self.table_constraints.is_empty();
        let mut summary = TableChangeSummary {
            properties_changed,
            ..TableChangeSummary::default()
        };
        for column in &self.columns {
            match column.state {
                DraftRowState::Added => {
                    if !is_empty_added_column(column) {
                        summary.added_columns += 1;
                    }
                }
                DraftRowState::Existing { .. } | DraftRowState::Removed { .. } => {}
            }
        }
        summary
    }

    pub fn change_summary(
        &self,
        baseline: Option<&crate::db::catalog_mutation::TableDefinition>,
    ) -> TableChangeSummary {
        let Some(baseline) = baseline else {
            return self.create_change_summary();
        };
        let baseline_columns = baseline
            .columns
            .iter()
            .map(|column| {
                (
                    column.name.as_str(),
                    column_signature(
                        &column.name,
                        &column.native_type,
                        column.nullable,
                        optional_string(&column.default_expression),
                        optional_bool(&column.identity),
                        optional_string(&column.generated_expression),
                        optional_string(&column.collation),
                        optional_string(&column.comment),
                    ),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        let mut summary = TableChangeSummary {
            properties_changed: self.name.value() != baseline.name
                || self.schema.value() != baseline.schema
                || self.owner.value() != baseline.owner
                || self.comment.value() != optional_string(&baseline.comment)
                || self.indexes != baseline.indexes
                || self.constraints != baseline.constraints
                || self.table_constraints.len() != baseline.structured_constraints.len()
                || self
                    .table_constraints
                    .iter()
                    .zip(&baseline.structured_constraints)
                    .any(|(draft, definition)| !table_constraint_matches(draft, definition)),
            ..TableChangeSummary::default()
        };
        let baseline_order = baseline
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>();
        let draft_order = self
            .columns
            .iter()
            .filter(|column| !matches!(column.state, DraftRowState::Removed { .. }))
            .filter_map(|column| column.existing_name.as_deref())
            .collect::<Vec<_>>();
        let baseline_surviving = baseline_order
            .iter()
            .copied()
            .filter(|name| draft_order.contains(name))
            .collect::<Vec<_>>();
        summary.column_order_changed = baseline_surviving != draft_order;
        for column in &self.columns {
            match &column.state {
                DraftRowState::Added => summary.added_columns += 1,
                DraftRowState::Removed { .. } => summary.removed_columns += 1,
                DraftRowState::Existing { .. } => {
                    let original = column
                        .existing_name
                        .as_deref()
                        .unwrap_or(column.name.value());
                    let before = baseline_columns.get(original).copied();
                    let after = column_signature(
                        column.name.value(),
                        column.native_type.value(),
                        column.nullable,
                        column.default_expression.value(),
                        column.identity,
                        column.generated_expression.value(),
                        column.collation.value(),
                        column.comment.value(),
                    );
                    if before != Some(after) {
                        summary.modified_columns += 1;
                    }
                }
            }
        }
        summary
    }

    pub fn is_dirty_against(
        &self,
        baseline: Option<&crate::db::catalog_mutation::TableDefinition>,
    ) -> bool {
        self.change_summary(baseline).is_dirty()
    }

    pub fn validation_focus(&self) -> Option<(Option<usize>, TableEditorFocus, String)> {
        if self.name.value().trim().is_empty() {
            return Some((
                None,
                TableEditorFocus::General(TableGeneralField::Name),
                "table name is required".into(),
            ));
        }
        if self.schema.value().trim().is_empty() {
            return Some((
                None,
                TableEditorFocus::General(TableGeneralField::Schema),
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
                    TableEditorFocus::ColumnDetails(TableColumnField::Name),
                    format!("column {} name is required", index + 1),
                ));
            }
            if column.native_type.value().trim().is_empty() {
                return Some((
                    Some(index),
                    TableEditorFocus::ColumnDetails(TableColumnField::Type),
                    format!("column {} type is required", index + 1),
                ));
            }
            if !names.insert(column.name.value().trim()) {
                return Some((
                    Some(index),
                    TableEditorFocus::ColumnDetails(TableColumnField::Name),
                    "column names must be unique".into(),
                ));
            }
            if column.identity && !column.default_expression.value().trim().is_empty() {
                return Some((
                    Some(index),
                    TableEditorFocus::ColumnDetails(TableColumnField::Identity),
                    format!("column {} identity cannot have a default", index + 1),
                ));
            }
        }
        (count == 0).then(|| {
            (
                None,
                TableEditorFocus::Columns,
                "a table requires at least one column".into(),
            )
        })
    }

    pub fn move_column(&mut self, delta: isize) {
        let last = self.columns.len().saturating_sub(1) as isize;
        self.selected_column = (self.selected_column as isize + delta).clamp(0, last) as usize;
    }

    pub fn reorder_selected_column(&mut self, delta: isize) -> bool {
        if self.column_editor.is_some() || delta == 0 {
            return false;
        }
        let Some(column) = self.columns.get(self.selected_column) else {
            return false;
        };
        if matches!(column.state, DraftRowState::Removed { .. }) {
            return false;
        }
        let direction = delta.signum();
        let mut target = self.selected_column as isize + direction;
        while target >= 0 && (target as usize) < self.columns.len() {
            if !matches!(
                self.columns[target as usize].state,
                DraftRowState::Removed { .. }
            ) {
                let target = target as usize;
                self.columns.swap(self.selected_column, target);
                self.selected_column = target;
                return true;
            }
            target += direction;
        }
        false
    }

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            TableEditorFocus::General(TableGeneralField::Name) => {
                TableEditorFocus::General(TableGeneralField::Schema)
            }
            TableEditorFocus::General(TableGeneralField::Schema) => {
                TableEditorFocus::General(TableGeneralField::Owner)
            }
            TableEditorFocus::General(TableGeneralField::Owner) => {
                TableEditorFocus::General(TableGeneralField::Comment)
            }
            TableEditorFocus::General(TableGeneralField::Comment) => {
                if self.database_kind == crate::profile::DatabaseKind::ClickHouse {
                    TableEditorFocus::General(TableGeneralField::ClickHouseEngine)
                } else if self.columns.is_empty() {
                    TableEditorFocus::Action(TableActionField::AddColumn)
                } else {
                    self.selected_column = self.selected_column.min(self.columns.len() - 1);
                    TableEditorFocus::Columns
                }
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseEngine) => {
                TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy) => {
                TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy)
            }
            TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy) => {
                TableEditorFocus::General(TableGeneralField::ClickHouseTtl)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseTtl) => {
                if self.columns.is_empty() {
                    TableEditorFocus::Action(TableActionField::AddColumn)
                } else {
                    self.selected_column = self.selected_column.min(self.columns.len() - 1);
                    TableEditorFocus::Columns
                }
            }
            TableEditorFocus::Columns => {
                if self.selected_column + 1 < self.columns.len() {
                    self.selected_column += 1;
                    TableEditorFocus::Columns
                } else {
                    TableEditorFocus::Action(TableActionField::AddColumn)
                }
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Name) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Type)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Type) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Default)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Default) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Comment)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Comment) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Nullable)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Nullable) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Identity)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Identity) => {
                if self.column_editor.is_some() {
                    TableEditorFocus::ColumnDetails(TableColumnField::Name)
                } else {
                    TableEditorFocus::Columns
                }
            }
            TableEditorFocus::Action(TableActionField::AddColumn) => {
                TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey)
            }
            TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey) => {
                TableEditorFocus::Action(TableActionField::AddConstraintUnique)
            }
            TableEditorFocus::Action(TableActionField::AddConstraintUnique) => {
                TableEditorFocus::Action(TableActionField::AddConstraintCheck)
            }
            TableEditorFocus::Action(TableActionField::AddConstraintCheck) => {
                TableEditorFocus::Action(TableActionField::AddConstraintForeignKey)
            }
            TableEditorFocus::Action(TableActionField::AddConstraintForeignKey) => {
                TableEditorFocus::Action(TableActionField::Review)
            }
            TableEditorFocus::Action(TableActionField::Constraint(index)) => {
                TableEditorFocus::Action(TableActionField::RemoveConstraint(index))
            }
            TableEditorFocus::Action(TableActionField::RemoveConstraint(index)) => {
                if index + 1 < self.table_constraints.len() {
                    TableEditorFocus::Action(TableActionField::Constraint(index + 1))
                } else {
                    TableEditorFocus::Action(TableActionField::Review)
                }
            }
            TableEditorFocus::Action(TableActionField::Review) => {
                TableEditorFocus::Action(TableActionField::Cancel)
            }
            TableEditorFocus::Action(TableActionField::Cancel) => self.focus,
        };
    }

    pub fn focus_next_action(&mut self, allow_structured_constraints: bool) {
        loop {
            self.focus_next();
            if allow_structured_constraints
                || !matches!(
                    self.focus,
                    TableEditorFocus::Action(
                        TableActionField::AddConstraintPrimaryKey
                            | TableActionField::AddConstraintUnique
                            | TableActionField::AddConstraintCheck
                            | TableActionField::AddConstraintForeignKey
                            | TableActionField::Constraint(_)
                            | TableActionField::RemoveConstraint(_)
                    )
                )
            {
                break;
            }
        }
    }

    pub fn focus_previous_action(&mut self, allow_structured_constraints: bool) {
        loop {
            self.focus_previous();
            if allow_structured_constraints
                || !matches!(
                    self.focus,
                    TableEditorFocus::Action(
                        TableActionField::AddConstraintPrimaryKey
                            | TableActionField::AddConstraintUnique
                            | TableActionField::AddConstraintCheck
                            | TableActionField::AddConstraintForeignKey
                            | TableActionField::Constraint(_)
                            | TableActionField::RemoveConstraint(_)
                    )
                )
            {
                break;
            }
        }
    }

    pub fn focus_previous(&mut self) {
        if matches!(
            self.focus,
            TableEditorFocus::Action(TableActionField::AddColumn)
        ) && self.columns.is_empty()
        {
            self.focus = TableEditorFocus::General(TableGeneralField::Comment);
            return;
        }
        self.focus = match self.focus {
            TableEditorFocus::General(TableGeneralField::Name) => self.focus,
            TableEditorFocus::General(TableGeneralField::Schema) => {
                TableEditorFocus::General(TableGeneralField::Name)
            }
            TableEditorFocus::General(TableGeneralField::Owner) => {
                TableEditorFocus::General(TableGeneralField::Schema)
            }
            TableEditorFocus::General(TableGeneralField::Comment) => {
                TableEditorFocus::General(TableGeneralField::Owner)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseEngine) => {
                TableEditorFocus::General(TableGeneralField::Comment)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy) => {
                TableEditorFocus::General(TableGeneralField::ClickHouseEngine)
            }
            TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy) => {
                TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseTtl) => {
                TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy)
            }
            TableEditorFocus::Columns => {
                if self.selected_column > 0 {
                    self.selected_column -= 1;
                    TableEditorFocus::Columns
                } else {
                    TableEditorFocus::General(TableGeneralField::Comment)
                }
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Name) => {
                if self.column_editor.is_some() {
                    TableEditorFocus::ColumnDetails(TableColumnField::Identity)
                } else {
                    TableEditorFocus::Columns
                }
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Type) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Name)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Default) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Type)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Comment) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Default)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Nullable) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Comment)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Identity) => {
                TableEditorFocus::ColumnDetails(TableColumnField::Nullable)
            }
            TableEditorFocus::Action(TableActionField::AddColumn) => {
                if self.columns.is_empty() {
                    TableEditorFocus::General(TableGeneralField::Comment)
                } else {
                    self.selected_column = self.columns.len() - 1;
                    TableEditorFocus::Columns
                }
            }
            TableEditorFocus::Action(TableActionField::Review) => {
                if let Some(index) = self.table_constraints.len().checked_sub(1) {
                    TableEditorFocus::Action(TableActionField::RemoveConstraint(index))
                } else {
                    TableEditorFocus::Action(TableActionField::AddConstraintForeignKey)
                }
            }
            TableEditorFocus::Action(
                TableActionField::AddConstraintForeignKey
                | TableActionField::AddConstraintCheck
                | TableActionField::AddConstraintUnique
                | TableActionField::AddConstraintPrimaryKey,
            ) => match self.focus {
                TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey) => {
                    TableEditorFocus::Action(TableActionField::AddColumn)
                }
                TableEditorFocus::Action(TableActionField::AddConstraintUnique) => {
                    TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey)
                }
                TableEditorFocus::Action(TableActionField::AddConstraintCheck) => {
                    TableEditorFocus::Action(TableActionField::AddConstraintUnique)
                }
                _ => TableEditorFocus::Action(TableActionField::AddConstraintCheck),
            },
            TableEditorFocus::Action(TableActionField::Constraint(index)) => {
                if index > 0 {
                    TableEditorFocus::Action(TableActionField::RemoveConstraint(index - 1))
                } else {
                    TableEditorFocus::Action(TableActionField::AddColumn)
                }
            }
            TableEditorFocus::Action(TableActionField::RemoveConstraint(index)) => {
                if index > 0 {
                    TableEditorFocus::Action(TableActionField::Constraint(index - 1))
                } else {
                    TableEditorFocus::Action(TableActionField::AddConstraintForeignKey)
                }
            }
            TableEditorFocus::Action(TableActionField::Cancel) => {
                TableEditorFocus::Action(TableActionField::Review)
            }
        };
    }

    pub fn begin_edit_selected_column(&mut self) -> bool {
        if self.column_editor.is_some() {
            return false;
        }
        let Some(column) = self.selected_column().cloned() else {
            return false;
        };
        self.column_editor = Some(TableColumnEditSession {
            target: TableColumnEditTarget::Existing {
                index: self.selected_column,
            },
            draft: column,
            error: None,
        });
        self.focus = TableEditorFocus::ColumnDetails(TableColumnField::Name);
        true
    }

    pub fn begin_add_column_above(&mut self) {
        let insert_at = self.selected_column.min(self.columns.len());
        self.begin_add_column_at(insert_at);
    }

    pub fn begin_add_column_below(&mut self) {
        let insert_at = self
            .selected_column
            .min(self.columns.len().saturating_sub(1))
            .saturating_add(1);
        self.begin_add_column_at(insert_at);
    }

    fn begin_add_column_at(&mut self, insert_at: usize) {
        if self.column_editor.is_some() {
            return;
        }
        let mut column = ColumnDraft::new_added_for_database(self.database_kind);
        column.ordinal_position = self
            .columns
            .iter()
            .map(|column| column.ordinal_position)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.column_editor = Some(TableColumnEditSession {
            target: TableColumnEditTarget::New { insert_at },
            draft: column,
            error: None,
        });
        self.focus = TableEditorFocus::ColumnDetails(TableColumnField::Name);
    }

    pub fn confirm_column_details(&mut self) -> bool {
        let Some(session) = self.column_editor.take() else {
            return false;
        };
        match session.target {
            TableColumnEditTarget::Existing { index } => {
                let Some(column) = self.columns.get_mut(index) else {
                    self.focus = TableEditorFocus::Columns;
                    return false;
                };
                *column = session.draft;
                self.selected_column = index;
            }
            TableColumnEditTarget::New { insert_at } => {
                let index = insert_at.min(self.columns.len());
                self.columns.insert(index, session.draft);
                self.selected_column = index;
            }
        }
        self.focus = TableEditorFocus::Columns;
        true
    }

    pub fn validate_column_details(&self) -> Option<(TableColumnField, String)> {
        let session = self.column_editor.as_ref()?;
        let column = &session.draft;
        if column.name.value().trim().is_empty() {
            return Some((TableColumnField::Name, "column name is required".into()));
        }
        if column.native_type.value().trim().is_empty() {
            return Some((TableColumnField::Type, "column type is required".into()));
        }
        if let Some(issue) =
            crate::db::column_type::ColumnTypePolicy::for_database(self.database_kind)
                .validate_native_type(column.native_type.value())
            && issue.kind == crate::db::column_type::ColumnTypeIssueKind::Invalid
        {
            let suggestion = issue
                .suggestion
                .map(|value| format!("; {value}"))
                .unwrap_or_default();
            return Some((
                TableColumnField::Type,
                format!("{}{}", issue.message, suggestion),
            ));
        }
        let target_index = match session.target {
            TableColumnEditTarget::Existing { index } => Some(index),
            TableColumnEditTarget::New { .. } => None,
        };
        if self.columns.iter().enumerate().any(|(index, existing)| {
            Some(index) != target_index
                && !matches!(existing.state, DraftRowState::Removed { .. })
                && existing.name.value().trim() == column.name.value().trim()
        }) {
            return Some((TableColumnField::Name, "column names must be unique".into()));
        }
        if column.identity && !column.default_expression.value().trim().is_empty() {
            return Some((
                TableColumnField::Identity,
                "identity cannot have a default; clear Default first".into(),
            ));
        }
        None
    }

    pub fn cancel_column_details(&mut self) {
        self.column_editor = None;
        if matches!(self.focus, TableEditorFocus::ColumnDetails(_)) {
            self.focus = TableEditorFocus::Columns;
        }
    }

    pub fn move_field(&mut self, delta: isize) {
        if self.constraint_editor.is_some() {
            self.move_table_constraint_details_field(delta);
            return;
        }
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                self.focus_previous();
            } else {
                self.focus_next();
            }
        }
    }

    pub fn selected_column(&self) -> Option<&ColumnDraft> {
        self.columns.get(self.selected_column)
    }

    pub fn selected_column_mut(&mut self) -> Option<&mut ColumnDraft> {
        self.columns.get_mut(self.selected_column)
    }

    pub fn remove_selected_column(&mut self) {
        if self.columns.get(self.selected_column).is_none() {
            return;
        }
        if self
            .columns
            .iter()
            .filter(|column| !matches!(column.state, DraftRowState::Removed { .. }))
            .count()
            <= 1
        {
            self.focus = TableEditorFocus::Columns;
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
        self.focus = TableEditorFocus::Columns;
    }

    pub fn restore_selected_column(&mut self) -> bool {
        let Some(column) = self.columns.get_mut(self.selected_column) else {
            return false;
        };
        let DraftRowState::Removed { id } = &column.state else {
            return false;
        };
        column.state = DraftRowState::Existing { id: id.clone() };
        self.focus = TableEditorFocus::Columns;
        true
    }

    pub fn selected_column_is_removed(&self) -> bool {
        self.selected_column()
            .is_some_and(|column| matches!(column.state, DraftRowState::Removed { .. }))
    }

    /// Remove the column identified by its stable draft row id.
    ///
    /// Unlike `remove_selected_column`, this is used by row-level mouse
    /// controls and therefore must never fall back to the current selection.
    pub fn remove_column_row(&mut self, row_id: uuid::Uuid) -> bool {
        let Some(index) = self
            .columns
            .iter()
            .position(|column| column.row_id == row_id)
        else {
            return false;
        };
        if self.column_editor.is_some()
            || matches!(self.columns[index].state, DraftRowState::Removed { .. })
            || self
                .columns
                .iter()
                .filter(|column| !matches!(column.state, DraftRowState::Removed { .. }))
                .count()
                <= 1
        {
            return false;
        }
        self.selected_column = index;
        self.focus = TableEditorFocus::Columns;
        self.remove_selected_column();
        true
    }

    /// Restore a previously removed column by its stable draft row id.
    pub fn restore_column_row(&mut self, row_id: uuid::Uuid) -> bool {
        let Some(index) = self
            .columns
            .iter()
            .position(|column| column.row_id == row_id)
        else {
            return false;
        };
        if self.column_editor.is_some()
            || !matches!(self.columns[index].state, DraftRowState::Removed { .. })
        {
            return false;
        }
        self.selected_column = index;
        self.focus = TableEditorFocus::Columns;
        self.restore_selected_column()
    }

    pub fn toggle_selected_column_nullable(&mut self) {
        if let Some(column) = self
            .column_editor
            .as_mut()
            .map(|session| &mut session.draft)
        {
            column.nullable = !column.nullable;
        }
    }

    pub fn toggle_selected_column_identity(&mut self) {
        if let Some(column) = self
            .column_editor
            .as_mut()
            .map(|session| &mut session.draft)
        {
            column.identity = !column.identity;
        }
    }

    pub(crate) fn input_for_target(&self, target: &TableEditorFocus) -> Option<&TextInput> {
        match target {
            TableEditorFocus::General(TableGeneralField::Name) => Some(&self.name),
            TableEditorFocus::General(TableGeneralField::Schema) => Some(&self.schema),
            TableEditorFocus::General(TableGeneralField::Owner) => Some(&self.owner),
            TableEditorFocus::General(TableGeneralField::Comment) => Some(&self.comment),
            TableEditorFocus::General(TableGeneralField::ClickHouseEngine) => {
                Some(&self.clickhouse_engine)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy) => {
                Some(&self.clickhouse_order_by)
            }
            TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy) => {
                Some(&self.clickhouse_partition_by)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseTtl) => {
                Some(&self.clickhouse_ttl)
            }
            TableEditorFocus::ColumnDetails(field) => {
                self.column_editor.as_ref().and_then(|session| match field {
                    TableColumnField::Name => Some(&session.draft.name),
                    TableColumnField::Type => Some(&session.draft.native_type),
                    TableColumnField::Default => Some(&session.draft.default_expression),
                    TableColumnField::Comment => Some(&session.draft.comment),
                    TableColumnField::Nullable | TableColumnField::Identity => None,
                })
            }
            TableEditorFocus::Action(TableActionField::Constraint(index)) => self
                .table_constraints
                .get(*index)
                .and_then(|constraint| match constraint.selected_field {
                    0 => Some(&constraint.name),
                    1 => match constraint.kind {
                        crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                            Some(&constraint.expression)
                        }
                        _ => Some(&constraint.columns),
                    },
                    2 if matches!(
                        constraint.kind,
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    ) =>
                    {
                        Some(&constraint.referenced_schema)
                    }
                    3 if matches!(
                        constraint.kind,
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    ) =>
                    {
                        Some(&constraint.referenced_relation)
                    }
                    4 if matches!(
                        constraint.kind,
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    ) =>
                    {
                        Some(&constraint.referenced_columns)
                    }
                    5 if matches!(
                        constraint.kind,
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    ) =>
                    {
                        Some(&constraint.on_update)
                    }
                    6 if matches!(
                        constraint.kind,
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
                    ) =>
                    {
                        Some(&constraint.on_delete)
                    }
                    _ => None,
                }),
            _ => None,
        }
    }

    pub(crate) fn input_for_target_mut(
        &mut self,
        target: &TableEditorFocus,
    ) -> Option<&mut TextInput> {
        match target {
            TableEditorFocus::General(TableGeneralField::Name) => Some(&mut self.name),
            TableEditorFocus::General(TableGeneralField::Schema) => Some(&mut self.schema),
            TableEditorFocus::General(TableGeneralField::Owner) => Some(&mut self.owner),
            TableEditorFocus::General(TableGeneralField::Comment) => Some(&mut self.comment),
            TableEditorFocus::General(TableGeneralField::ClickHouseEngine) => {
                Some(&mut self.clickhouse_engine)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy) => {
                Some(&mut self.clickhouse_order_by)
            }
            TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy) => {
                Some(&mut self.clickhouse_partition_by)
            }
            TableEditorFocus::General(TableGeneralField::ClickHouseTtl) => {
                Some(&mut self.clickhouse_ttl)
            }
            TableEditorFocus::ColumnDetails(TableColumnField::Name) => self
                .column_editor
                .as_mut()
                .map(|session| &mut session.draft.name),
            TableEditorFocus::ColumnDetails(TableColumnField::Type) => self
                .column_editor
                .as_mut()
                .map(|session| &mut session.draft.native_type),
            TableEditorFocus::ColumnDetails(TableColumnField::Default) => self
                .column_editor
                .as_mut()
                .map(|session| &mut session.draft.default_expression),
            TableEditorFocus::ColumnDetails(TableColumnField::Comment) => self
                .column_editor
                .as_mut()
                .map(|session| &mut session.draft.comment),
            TableEditorFocus::Action(TableActionField::Constraint(index)) => self
                .table_constraints
                .get_mut(*index)
                .and_then(ConstraintDraft::selected_input_mut),
            _ => None,
        }
    }

    pub(crate) fn selected_text_input_mut(&mut self) -> Option<&mut TextInput> {
        if self.constraint_editor.is_some() {
            return self
                .constraint_editor
                .as_mut()
                .and_then(|session| session.draft.selected_input_mut());
        }
        let focus = self.focus;
        self.input_for_target_mut(&focus)
    }

    pub(crate) fn constraint_details_input_for_field(&self, field: usize) -> Option<&TextInput> {
        self.constraint_editor
            .as_ref()
            .and_then(|session| session.draft.input_for_field(field))
    }

    pub(crate) fn constraint_details_input_for_field_mut(
        &mut self,
        field: usize,
    ) -> Option<&mut TextInput> {
        self.constraint_editor
            .as_mut()
            .and_then(|session| session.draft.input_for_field_mut(field))
    }

    pub fn insert(&mut self, character: char) {
        if let Some(input) = self.selected_text_input_mut() {
            input.insert(character);
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.selected_text_input_mut() {
            input.paste(text);
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

    pub fn undo(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.undo();
        }
    }

    pub fn redo(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.redo();
        }
    }

    pub(crate) fn finish_edit_group(&mut self) {
        if let Some(input) = self.selected_text_input_mut() {
            input.finish_edit_group();
        }
    }
}

fn clickhouse_order_by_clause(ddl: &str) -> Option<String> {
    let upper = ddl.to_ascii_uppercase();
    let marker = upper.find(" ORDER BY ")? + " ORDER BY ".len();
    let tail = &ddl[marker..];
    let upper_tail = tail.to_ascii_uppercase();
    let end = [
        " PARTITION BY ",
        " PRIMARY KEY ",
        " SAMPLE BY ",
        " TTL ",
        " SETTINGS ",
        " COMMENT ",
    ]
    .iter()
    .filter_map(|clause| upper_tail.find(clause))
    .min()
    .unwrap_or(tail.len());
    Some(tail[..end].trim().to_owned()).filter(|value| !value.is_empty())
}

pub(crate) fn table_constraint_matches(
    draft: &ConstraintDraft,
    definition: &crate::db::catalog_mutation::ConstraintDefinition,
) -> bool {
    let restored = ConstraintDraft::from_definition(definition);
    draft.name == restored.name
        && draft.kind == restored.kind
        && draft.columns == restored.columns
        && draft.referenced_schema == restored.referenced_schema
        && draft.referenced_relation == restored.referenced_relation
        && draft.referenced_columns == restored.referenced_columns
        && draft.match_type == restored.match_type
        && draft.on_update == restored.on_update
        && draft.on_delete == restored.on_delete
        && draft.expression == restored.expression
        && draft.no_inherit == restored.no_inherit
        && draft.deferrable == restored.deferrable
        && draft.initially_deferred == restored.initially_deferred
        && draft.not_valid == restored.not_valid
}

fn optional_string(value: &crate::db::catalog::OptionalMetadata<String>) -> &str {
    match value {
        crate::db::catalog::OptionalMetadata::Supported(Some(value)) => value,
        _ => "",
    }
}

fn optional_bool(value: &crate::db::catalog::OptionalMetadata<bool>) -> bool {
    matches!(
        value,
        crate::db::catalog::OptionalMetadata::Supported(Some(true))
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ColumnSignature<'a> {
    name: &'a str,
    native_type: &'a str,
    nullable: bool,
    default_expression: &'a str,
    identity: bool,
    generated_expression: &'a str,
    collation: &'a str,
    comment: &'a str,
}

#[allow(clippy::too_many_arguments)]
fn column_signature<'a>(
    name: &'a str,
    native_type: &'a str,
    nullable: bool,
    default_expression: &'a str,
    identity: bool,
    generated_expression: &'a str,
    collation: &'a str,
    comment: &'a str,
) -> ColumnSignature<'a> {
    ColumnSignature {
        name,
        native_type,
        nullable,
        default_expression,
        identity,
        generated_expression,
        collation,
        comment,
    }
}

fn is_empty_added_column(column: &ColumnDraft) -> bool {
    column.name.value().is_empty()
        && column.native_type.value() == "text"
        && column.nullable
        && column.default_expression.value().is_empty()
        && !column.identity
        && column.generated_expression.value().is_empty()
        && column.collation.value().is_empty()
        && column.comment.value().is_empty()
}

impl ColumnDraft {
    pub fn field_changed_against(
        &self,
        field: TableColumnField,
        baseline: Option<&crate::db::catalog_mutation::TableDefinition>,
    ) -> bool {
        if !matches!(&self.state, DraftRowState::Existing { .. }) {
            return false;
        }
        let original_name = self.existing_name.as_deref().unwrap_or(self.name.value());
        let Some(before) = baseline.and_then(|table| {
            table
                .columns
                .iter()
                .find(|column| column.name == original_name)
        }) else {
            return false;
        };
        match field {
            TableColumnField::Name => self.name.value() != before.name,
            TableColumnField::Type => self.native_type.value() != before.native_type,
            TableColumnField::Default => {
                self.default_expression.value() != optional_string(&before.default_expression)
            }
            TableColumnField::Comment => self.comment.value() != optional_string(&before.comment),
            TableColumnField::Nullable => self.nullable != before.nullable,
            TableColumnField::Identity => self.identity != optional_bool(&before.identity),
        }
    }

    pub fn new_added() -> Self {
        Self::new_added_for_database(crate::profile::DatabaseKind::Postgres)
    }

    pub fn new_added_for_database(database_kind: crate::profile::DatabaseKind) -> Self {
        Self {
            row_id: Uuid::new_v4(),
            ordinal_position: 1,
            existing_name: None,
            name: TextInput::default(),
            native_type: crate::db::column_type::ColumnTypePolicy::for_database(database_kind)
                .default_native_type()
                .into(),
            nullable: database_kind != crate::profile::DatabaseKind::ClickHouse,
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

#[allow(clippy::large_enum_variant)]
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
    pub fn input_for_target(
        &self,
        target: &crate::action::CatalogEditorCursorTarget,
    ) -> Option<&TextInput> {
        match (self, target) {
            (Self::Schema(draft), crate::action::CatalogEditorCursorTarget::SchemaField(index)) => {
                match index {
                    0 => Some(&draft.name),
                    1 => Some(&draft.owner),
                    2 => Some(&draft.comment),
                    _ => None,
                }
            }
            (Self::View(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                match field {
                    CatalogFormFocus::Name => Some(&draft.name),
                    CatalogFormFocus::Schema => Some(&draft.schema),
                    CatalogFormFocus::Owner => Some(&draft.owner),
                    CatalogFormFocus::Comment => Some(&draft.comment),
                    CatalogFormFocus::OutputColumns => Some(&draft.output_columns),
                    CatalogFormFocus::Query => Some(&draft.query),
                    _ => None,
                }
            }
            (
                Self::MaterializedView(draft),
                crate::action::CatalogEditorCursorTarget::FormField(field),
            ) => match field {
                CatalogFormFocus::Name => Some(&draft.name),
                CatalogFormFocus::Schema => Some(&draft.schema),
                CatalogFormFocus::Owner => Some(&draft.owner),
                CatalogFormFocus::Comment => Some(&draft.comment),
                CatalogFormFocus::Query if draft.query_editable => Some(&draft.query),
                CatalogFormFocus::Tablespace => Some(&draft.tablespace),
                _ => None,
            },
            (Self::Sequence(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                match field {
                    CatalogFormFocus::Name => Some(&draft.name),
                    CatalogFormFocus::Schema => Some(&draft.schema),
                    CatalogFormFocus::Owner => Some(&draft.owner),
                    CatalogFormFocus::Comment => Some(&draft.comment),
                    CatalogFormFocus::DataType => Some(&draft.data_type),
                    CatalogFormFocus::Increment => Some(&draft.increment),
                    CatalogFormFocus::MinValue
                        if draft.min_value.kind == SequenceBoundKind::Custom =>
                    {
                        Some(&draft.min_value.value)
                    }
                    CatalogFormFocus::MaxValue
                        if draft.max_value.kind == SequenceBoundKind::Custom =>
                    {
                        Some(&draft.max_value.value)
                    }
                    CatalogFormFocus::StartValue => Some(&draft.start_value),
                    CatalogFormFocus::RestartValue => Some(&draft.restart_value),
                    CatalogFormFocus::Cache => Some(&draft.cache),
                    CatalogFormFocus::OwnedBy => Some(&draft.owned_by),
                    _ => None,
                }
            }
            (Self::Database(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                database_input(draft, *field)
            }
            (Self::Role(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                role_input(draft, *field)
            }
            (Self::Table(draft), crate::action::CatalogEditorCursorTarget::TableField(field)) => {
                draft.input_for_target(field)
            }
            (
                Self::Table(draft),
                crate::action::CatalogEditorCursorTarget::ConstraintField(field),
            ) => draft.constraint_details_input_for_field(*field),
            _ => None,
        }
    }

    pub fn input_for_target_mut(
        &mut self,
        target: &crate::action::CatalogEditorCursorTarget,
    ) -> Option<&mut TextInput> {
        match (self, target) {
            (Self::Schema(draft), crate::action::CatalogEditorCursorTarget::SchemaField(index)) => {
                match index {
                    0 => Some(&mut draft.name),
                    1 => Some(&mut draft.owner),
                    2 => Some(&mut draft.comment),
                    _ => None,
                }
            }
            (Self::View(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                match field {
                    CatalogFormFocus::Name => Some(&mut draft.name),
                    CatalogFormFocus::Schema => Some(&mut draft.schema),
                    CatalogFormFocus::Owner => Some(&mut draft.owner),
                    CatalogFormFocus::Comment => Some(&mut draft.comment),
                    CatalogFormFocus::OutputColumns => Some(&mut draft.output_columns),
                    CatalogFormFocus::Query => Some(&mut draft.query),
                    _ => None,
                }
            }
            (
                Self::MaterializedView(draft),
                crate::action::CatalogEditorCursorTarget::FormField(field),
            ) => match field {
                CatalogFormFocus::Name => Some(&mut draft.name),
                CatalogFormFocus::Schema => Some(&mut draft.schema),
                CatalogFormFocus::Owner => Some(&mut draft.owner),
                CatalogFormFocus::Comment => Some(&mut draft.comment),
                CatalogFormFocus::Query if draft.query_editable => Some(&mut draft.query),
                CatalogFormFocus::Tablespace => Some(&mut draft.tablespace),
                _ => None,
            },
            (Self::Sequence(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                match field {
                    CatalogFormFocus::Name => Some(&mut draft.name),
                    CatalogFormFocus::Schema => Some(&mut draft.schema),
                    CatalogFormFocus::Owner => Some(&mut draft.owner),
                    CatalogFormFocus::Comment => Some(&mut draft.comment),
                    CatalogFormFocus::DataType => Some(&mut draft.data_type),
                    CatalogFormFocus::Increment => Some(&mut draft.increment),
                    CatalogFormFocus::MinValue
                        if draft.min_value.kind == SequenceBoundKind::Custom =>
                    {
                        Some(&mut draft.min_value.value)
                    }
                    CatalogFormFocus::MaxValue
                        if draft.max_value.kind == SequenceBoundKind::Custom =>
                    {
                        Some(&mut draft.max_value.value)
                    }
                    CatalogFormFocus::StartValue => Some(&mut draft.start_value),
                    CatalogFormFocus::RestartValue => Some(&mut draft.restart_value),
                    CatalogFormFocus::Cache => Some(&mut draft.cache),
                    CatalogFormFocus::OwnedBy => Some(&mut draft.owned_by),
                    _ => None,
                }
            }
            (Self::Database(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                database_input_mut(draft, *field)
            }
            (Self::Role(draft), crate::action::CatalogEditorCursorTarget::FormField(field)) => {
                role_input_mut(draft, *field)
            }
            (Self::Table(draft), crate::action::CatalogEditorCursorTarget::TableField(field)) => {
                draft.input_for_target_mut(field)
            }
            (
                Self::Table(draft),
                crate::action::CatalogEditorCursorTarget::ConstraintField(field),
            ) => draft.constraint_details_input_for_field_mut(*field),
            _ => None,
        }
    }

    pub fn validation_focus(&self) -> Option<CatalogFormFocus> {
        match self {
            Self::View(draft) => draft.validation_focus(),
            Self::MaterializedView(draft) => draft.validation_focus(),
            Self::Sequence(draft) => draft.validation_focus(),
            _ => None,
        }
    }

    pub fn focus_kind(&self, create_mode: bool) -> CatalogFormFocusKind {
        if self.focus_is_toggle(create_mode) {
            return CatalogFormFocusKind::Toggle;
        }
        if matches!(
            self,
            Self::Database(draft) if !draft.focus_enabled(draft.focus)
        ) {
            return CatalogFormFocusKind::Disabled;
        }
        if self.focused_action().is_some() {
            return CatalogFormFocusKind::Action;
        }
        if self.focus_is_choice() {
            return CatalogFormFocusKind::Choice;
        }
        if !self.focus_accepts_text() {
            return match self {
                Self::Role(_) => CatalogFormFocusKind::ReadOnly,
                Self::MaterializedView(draft) if draft.focus == CatalogFormFocus::Query => {
                    CatalogFormFocusKind::ReadOnly
                }
                Self::View(draft)
                    if matches!(
                        draft.focus,
                        CatalogFormFocus::SecurityBarrier
                            | CatalogFormFocus::SecurityInvoker
                            | CatalogFormFocus::CheckOption
                    ) =>
                {
                    CatalogFormFocusKind::Disabled
                }
                _ => CatalogFormFocusKind::ReadOnly,
            };
        }
        CatalogFormFocusKind::Text
    }
    pub fn owner(&self) -> Option<&TextInput> {
        match self {
            Self::Database(draft) => Some(&draft.owner),
            Self::Schema(draft) => Some(&draft.owner),
            Self::View(draft) => Some(&draft.owner),
            Self::MaterializedView(draft) => Some(&draft.owner),
            Self::Sequence(draft) => Some(&draft.owner),
            _ => None,
        }
    }

    pub fn owner_mut(&mut self) -> Option<&mut TextInput> {
        match self {
            Self::Database(draft) => Some(&mut draft.owner),
            Self::Schema(draft) => Some(&mut draft.owner),
            Self::View(draft) => Some(&mut draft.owner),
            Self::MaterializedView(draft) => Some(&mut draft.owner),
            Self::Sequence(draft) => Some(&mut draft.owner),
            _ => None,
        }
    }

    pub fn focus_accepts_text(&self) -> bool {
        match self {
            Self::Database(draft) => {
                matches!(
                    draft.focus,
                    CatalogFormFocus::Name
                        | CatalogFormFocus::Owner
                        | CatalogFormFocus::Comment
                        | CatalogFormFocus::Template
                        | CatalogFormFocus::Encoding
                        | CatalogFormFocus::LocaleProvider
                        | CatalogFormFocus::Locale
                        | CatalogFormFocus::Collation
                        | CatalogFormFocus::Ctype
                        | CatalogFormFocus::Tablespace
                        | CatalogFormFocus::ConnectionLimit
                ) && draft.focus_enabled(draft.focus)
            }
            Self::Role(draft) => matches!(
                draft.focus,
                CatalogFormFocus::Name
                    | CatalogFormFocus::Comment
                    | CatalogFormFocus::Password
                    | CatalogFormFocus::ValidUntil
                    | CatalogFormFocus::ConnectionLimit
                    | CatalogFormFocus::Memberships
            ),
            Self::View(draft) => matches!(
                draft.focus,
                CatalogFormFocus::Name
                    | CatalogFormFocus::Schema
                    | CatalogFormFocus::Owner
                    | CatalogFormFocus::Comment
                    | CatalogFormFocus::OutputColumns
                    | CatalogFormFocus::Query
            ),
            Self::MaterializedView(draft) => {
                matches!(
                    draft.focus,
                    CatalogFormFocus::Name
                        | CatalogFormFocus::Schema
                        | CatalogFormFocus::Owner
                        | CatalogFormFocus::Comment
                        | CatalogFormFocus::Tablespace
                ) || draft.focus == CatalogFormFocus::Query && draft.query_editable
            }
            Self::Sequence(draft) => {
                matches!(
                    draft.focus,
                    CatalogFormFocus::Name
                        | CatalogFormFocus::Schema
                        | CatalogFormFocus::Owner
                        | CatalogFormFocus::Comment
                        | CatalogFormFocus::DataType
                        | CatalogFormFocus::Increment
                        | CatalogFormFocus::StartValue
                        | CatalogFormFocus::RestartValue
                        | CatalogFormFocus::Cache
                        | CatalogFormFocus::OwnedBy
                ) || draft.focus == CatalogFormFocus::MinValue
                    && draft.min_value.kind == SequenceBoundKind::Custom
                    || draft.focus == CatalogFormFocus::MaxValue
                        && draft.max_value.kind == SequenceBoundKind::Custom
            }
            _ => true,
        }
    }

    pub fn focus_is_choice(&self) -> bool {
        match self {
            Self::View(draft) => match draft.focus {
                CatalogFormFocus::SecurityBarrier => {
                    draft.security_barrier.availability.is_available()
                }
                CatalogFormFocus::SecurityInvoker => {
                    draft.security_invoker.availability.is_available()
                }
                CatalogFormFocus::CheckOption => draft.check_option.availability.is_available(),
                _ => false,
            },
            Self::Sequence(draft) => {
                matches!(
                    draft.focus,
                    CatalogFormFocus::MinValue | CatalogFormFocus::MaxValue
                )
            }
            _ => false,
        }
    }

    pub fn focus_is_toggle(&self, create_mode: bool) -> bool {
        if matches!(
            self,
            Self::Database(draft)
                if matches!(draft.focus, CatalogFormFocus::AllowConnections | CatalogFormFocus::IsTemplate)
        ) || matches!(
            self,
            Self::Role(draft)
                if matches!(
                    draft.focus,
                    CatalogFormFocus::Login
                        | CatalogFormFocus::Superuser
                        | CatalogFormFocus::CreateDb
                        | CatalogFormFocus::CreateRole
                        | CatalogFormFocus::Inherit
                        | CatalogFormFocus::Replication
                        | CatalogFormFocus::BypassRls
                )
        ) {
            return true;
        }
        matches!(self, Self::Sequence(draft) if draft.focus == CatalogFormFocus::Cycle)
            || matches!(self, Self::MaterializedView(draft) if create_mode && draft.focus == CatalogFormFocus::WithData)
    }

    pub fn focused_action(&self) -> Option<CatalogFormFocus> {
        let focus = match self {
            Self::Database(draft) => draft.focus,
            Self::Role(draft) => draft.focus,
            Self::View(draft) => draft.focus,
            Self::MaterializedView(draft) => draft.focus,
            Self::Sequence(draft) => draft.focus,
            _ => return Some(CatalogFormFocus::Review),
        };
        matches!(focus, CatalogFormFocus::Review | CatalogFormFocus::Cancel).then_some(focus)
    }

    pub fn move_field(&mut self, delta: isize, allow_with_data: bool) {
        self.finish_edit_group();
        match self {
            Self::Table(d) => d.move_field(delta),
            Self::Schema(d) => d.move_field(delta),
            Self::View(d) => d.move_field(delta),
            Self::MaterializedView(d) => d.move_field(delta, allow_with_data),
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
    pub fn paste(&mut self, text: &str) {
        match self {
            Self::Database(draft) => draft.paste(text),
            Self::Role(draft) => draft.paste(text),
            Self::Table(draft) => draft.paste(text),
            Self::View(draft) => draft.paste(text),
            Self::MaterializedView(draft) => draft.paste(text),
            Self::Sequence(draft) => draft.paste(text),
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
            Self::Database(draft) => draft.delete_previous_word(),
            Self::Role(draft) => draft.delete_previous_word(),
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
            Self::Database(draft) => draft.delete_to_start(),
            Self::Role(draft) => draft.delete_to_start(),
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
            Self::Database(_) => {
                if let Some(input) = self.selected_input_mut() {
                    input.move_left();
                }
            }
            Self::Role(d) => d.move_left(),
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
            Self::Database(_) => {
                if let Some(input) = self.selected_input_mut() {
                    input.move_right();
                }
            }
            Self::Role(d) => d.move_right(),
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
            Self::Database(_) => {
                if let Some(input) = self.selected_input_mut() {
                    input.move_home();
                }
            }
            Self::Role(d) => d.move_home(),
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
            Self::Database(_) => {
                if let Some(input) = self.selected_input_mut() {
                    input.move_end();
                }
            }
            Self::Role(d) => d.move_end(),
            _ => {}
        }
    }

    pub fn undo(&mut self) {
        if let Self::Role(draft) = self
            && draft.focus == CatalogFormFocus::Password
        {
            draft.password_undo();
            return;
        }
        if let Some(input) = self.selected_input_mut() {
            input.undo();
        }
    }

    pub fn redo(&mut self) {
        if let Self::Role(draft) = self
            && draft.focus == CatalogFormFocus::Password
        {
            draft.password_redo();
            return;
        }
        if let Some(input) = self.selected_input_mut() {
            input.redo();
        }
    }

    pub fn finish_edit_group(&mut self) {
        if let Self::Role(draft) = self
            && draft.focus == CatalogFormFocus::Password
        {
            draft.password_finish_edit_group();
            return;
        }
        if let Some(input) = self.selected_input_mut() {
            input.finish_edit_group();
        }
    }

    pub(crate) fn selected_input_mut(&mut self) -> Option<&mut TextInput> {
        match self {
            Self::Table(draft) => draft.selected_text_input_mut(),
            Self::Schema(draft) => Some(draft.selected_input_mut()),
            Self::View(draft) => draft.selected_input_mut(),
            Self::MaterializedView(draft) => draft.selected_input_mut(),
            Self::Sequence(draft) => draft.selected_input_mut(),
            Self::Database(draft) => draft.selected_input_mut(),
            Self::Role(draft) => draft.selected_input_mut(),
            Self::Constraint(draft) => draft.selected_input_mut(),
            Self::Index(_) => None,
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
        self.filter.finish_edit_group();
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

    pub fn undo(&mut self, choices: &[CatalogOwnerChoice]) {
        self.filter.undo();
        self.reconcile(choices);
    }

    pub fn redo(&mut self, choices: &[CatalogOwnerChoice]) {
        self.filter.redo();
        self.reconcile(choices);
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
    pub database_kind: Option<crate::profile::DatabaseKind>,
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
    pub outcome_unknown: bool,
    pub owner_picker: OwnerPickerState,
    pub preview_scroll: usize,
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
            CatalogMutationAnchor::Principal(entry) => match entry.kind {
                crate::db::principal::PrincipalKind::User => CatalogObjectType::LoginRole,
                crate::db::principal::PrincipalKind::Role => CatalogObjectType::Role,
            },
            _ => options.first().map(|option| option.object_type).unwrap_or(
                CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Schema),
            ),
        });
        Self {
            database_kind: None,
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
            outcome_unknown: false,
            owner_picker: OwnerPickerState::default(),
            preview_scroll: 0,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.operation.is_some()
    }

    pub fn table_change_summary(&self) -> Option<TableChangeSummary> {
        let (CatalogDraft::Table(draft), baseline) = (self.draft.as_ref()?, self.baseline.as_ref())
        else {
            return None;
        };
        let baseline = match baseline {
            Some(CatalogObjectDefinition::Table(definition)) => Some(definition),
            _ => None,
        };
        Some(draft.change_summary(baseline))
    }

    pub fn clickhouse_table_change_summary(&self) -> Option<TableChangeSummary> {
        let CatalogDraft::Table(draft) = self.draft.as_ref()? else {
            return None;
        };
        let Some(CatalogObjectDefinition::ClickHouseTable(baseline)) = self.baseline.as_ref()
        else {
            return None;
        };
        Some(TableChangeSummary {
            properties_changed: draft.name.value() != baseline.name
                || draft.comment.value()
                    != draft
                        .clickhouse_baseline_comment
                        .as_deref()
                        .unwrap_or_default()
                || matches!(
                    &baseline.comment,
                    crate::db::catalog::OptionalMetadata::Unsupported
                ) && !draft.comment.value().is_empty(),
            added_columns: draft
                .columns
                .iter()
                .filter(|column| matches!(column.state, DraftRowState::Added))
                .count(),
            modified_columns: draft
                .columns
                .iter()
                .filter(|column| {
                    matches!(column.state, DraftRowState::Existing { .. })
                        && (column.name.value()
                            != column.existing_name.as_deref().unwrap_or_default()
                            || baseline
                                .columns
                                .iter()
                                .find(|base| {
                                    Some(base.name.as_str()) == column.existing_name.as_deref()
                                })
                                .is_some_and(|base| {
                                    base.native_type != column.native_type.value()
                                        || base.default_expression
                                            != column.default_expression.value()
                                        || match &base.comment {
                                            crate::db::catalog::OptionalMetadata::Unsupported => {
                                                true
                                            }
                                            crate::db::catalog::OptionalMetadata::Supported(
                                                value,
                                            ) => {
                                                value.as_deref().unwrap_or_default()
                                                    != column.comment.value()
                                            }
                                        }
                                }))
                })
                .count(),
            removed_columns: 0,
            ..TableChangeSummary::default()
        })
    }

    /// True when a catalog form focuses the owner field, which the owner picker owns.
    pub fn owner_field_focused(&self) -> bool {
        matches!(
            self.draft.as_ref(),
            Some(CatalogDraft::Schema(draft)) if draft.selected_field == SCHEMA_OWNER_FIELD
        ) || matches!(
            self.draft.as_ref(),
            Some(CatalogDraft::View(draft)) if draft.focus == CatalogFormFocus::Owner
        ) || matches!(
            self.draft.as_ref(),
            Some(CatalogDraft::MaterializedView(draft)) if draft.focus == CatalogFormFocus::Owner
        ) || matches!(
            self.draft.as_ref(),
            Some(CatalogDraft::Sequence(draft)) if draft.focus == CatalogFormFocus::Owner
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
            let draft = DatabaseDraft::new_for_database(
                self.database_kind
                    .unwrap_or(crate::profile::DatabaseKind::Postgres),
            );
            self.draft = Some(CatalogDraft::Database(draft));
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
                CatalogMutationAnchor::Principal(_) => None,
            },
            self.object_type,
        ) {
            if id.kind == crate::db::catalog::CatalogKind::Database
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Schema)
            {
                self.draft = Some(CatalogDraft::Schema(SchemaDraft::new()));
            }
            if matches!(
                id.kind,
                crate::db::catalog::CatalogKind::Database | crate::db::catalog::CatalogKind::Schema
            ) && object_type
                == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Table)
            {
                self.draft = Some(CatalogDraft::Table(TableDraft::new_for_database(
                    id.native_path
                        .get(1)
                        .or_else(|| id.native_path.first())
                        .cloned()
                        .unwrap_or_default(),
                    self.database_kind
                        .unwrap_or(crate::profile::DatabaseKind::Postgres),
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
                    min_value: crate::db::catalog_mutation::SequenceBound::Unset.into(),
                    max_value: crate::db::catalog_mutation::SequenceBound::Unset.into(),
                    start_value: "1".into(),
                    restart_value: TextInput::default(),
                    cache: "1".into(),
                    cycle: false,
                    owned_by: "NONE".into(),
                    focus: CatalogFormFocus::Name,
                }));
            }
            if id.kind == crate::db::catalog::CatalogKind::Database
                && object_type
                    == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::Database)
            {
                let mut draft = DatabaseDraft::new_for_database(
                    self.database_kind
                        .unwrap_or(crate::profile::DatabaseKind::Postgres),
                );
                draft.name = id.native_path.first().cloned().unwrap_or_default().into();
                self.draft = Some(CatalogDraft::Database(draft));
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
            if matches!(
                id.kind,
                crate::db::catalog::CatalogKind::Database | crate::db::catalog::CatalogKind::Schema
            ) && object_type == CatalogObjectType::Catalog(crate::db::catalog::CatalogKind::View)
            {
                self.draft = Some(CatalogDraft::View(ViewDraft {
                    name: TextInput::default(),
                    schema: id
                        .native_path
                        .get(1)
                        .or_else(|| id.native_path.first())
                        .cloned()
                        .unwrap_or_default()
                        .into(),
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
                    focus: CatalogFormFocus::Name,
                    clickhouse_native: None,
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
                    focus: CatalogFormFocus::Name,
                    query_editable: true,
                    clickhouse_native: None,
                }));
            }
        }
        self.error = None;
        self.outcome_unknown = false;
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
        if let Some(CatalogObjectDefinition::Table(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::Table(
                TableDraft::from_definition_for_database(
                    definition,
                    self.database_kind
                        .unwrap_or(crate::profile::DatabaseKind::Postgres),
                ),
            ));
        }
        if let Some(CatalogObjectDefinition::ClickHouseTable(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::Table(TableDraft::from_clickhouse_definition(
                definition,
            )));
        }
        if let Some(CatalogObjectDefinition::View(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::View(ViewDraft::from_definition(definition)));
        }
        if let Some(CatalogObjectDefinition::Sequence(definition)) = self.baseline.as_ref() {
            self.draft = Some(CatalogDraft::Sequence(SequenceDraft::from_definition(
                definition,
            )));
        }
        self.page = CatalogEditorPage::Form;
        true
    }

    pub fn accepts_definition_request(
        &self,
        request: &crate::db::catalog_mutation::CatalogObjectDefinitionRequest,
    ) -> bool {
        let anchor_matches = match &self.anchor {
            CatalogMutationAnchor::Catalog(object) => {
                object == &request.object && request.principal.is_none()
            }
            CatalogMutationAnchor::Principal(principal) => {
                request.principal.as_ref() == Some(principal)
                    && request.object.profile_id() == principal.id.profile_id
                    && request.object.kind == crate::db::catalog::CatalogKind::Database
                    && request.object.native_path.len() == 2
                    && request.object.native_path[0] == "__role__"
                    && request.object.native_path[1] == principal.name
            }
            _ => false,
        };
        self.operation
            == Some(CatalogEditorOperation::LoadingDefinition {
                request_id: request.request_id,
            })
            && self.catalog_epoch == request.catalog_epoch
            && anchor_matches
    }

    pub fn begin_planning(&mut self, request_id: u64) -> bool {
        if self.page != CatalogEditorPage::Form || self.is_busy() || self.draft.is_none() {
            return false;
        }
        self.error = None;
        self.operation = Some(CatalogEditorOperation::Planning { request_id });
        true
    }

    pub fn scroll_preview(&mut self, delta: isize, max_offset: usize) {
        let next = if delta < 0 {
            self.preview_scroll.saturating_sub(delta.unsigned_abs())
        } else {
            self.preview_scroll.saturating_add(delta as usize)
        };
        self.preview_scroll = next.min(max_offset);
    }

    pub fn preview_home(&mut self) {
        self.preview_scroll = 0;
    }

    pub fn preview_end(&mut self, max_offset: usize) {
        self.preview_scroll = max_offset;
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
        self.preview_scroll = 0;
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
        self.error = None;
        self.operation = Some(CatalogEditorOperation::Applying { request_id });
        true
    }

    pub fn apply_failed(&mut self, request_id: u64, message: impl Into<String>) -> bool {
        self.apply_failed_with_outcome(request_id, message, false)
    }

    pub fn apply_failed_with_outcome(
        &mut self,
        request_id: u64,
        message: impl Into<String>,
        outcome_unknown: bool,
    ) -> bool {
        if self.operation != Some(CatalogEditorOperation::Applying { request_id }) {
            return false;
        }
        self.operation = None;
        self.error = Some(message.into());
        self.outcome_unknown = outcome_unknown;
        self.page = CatalogEditorPage::SqlPreview;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::catalog_mutation::{SequenceBound, ViewOption};

    fn view_draft() -> ViewDraft {
        ViewDraft {
            name: TextInput::default(),
            schema: TextInput::default(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            query: TextInput::default(),
            output_columns: TextInput::default(),
            security_barrier: ViewOption::available(None),
            security_invoker: ViewOption::unavailable("not supported"),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::Name,
            clickhouse_native: None,
        }
    }

    fn materialized_view_draft(query_editable: bool) -> MaterializedViewDraft {
        MaterializedViewDraft {
            name: TextInput::default(),
            schema: TextInput::default(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            query: TextInput::default(),
            tablespace: TextInput::default(),
            with_data: true,
            focus: CatalogFormFocus::Name,
            query_editable,
            clickhouse_native: None,
        }
    }

    fn sequence_draft() -> SequenceDraft {
        SequenceDraft {
            name: TextInput::default(),
            schema: TextInput::default(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            data_type: TextInput::default(),
            increment: TextInput::default(),
            min_value: SequenceBound::Unset.into(),
            max_value: SequenceBound::Unset.into(),
            start_value: TextInput::default(),
            restart_value: TextInput::default(),
            cache: TextInput::default(),
            cycle: false,
            owned_by: TextInput::default(),
            focus: CatalogFormFocus::Name,
        }
    }

    #[test]
    fn catalog_form_focus_view_order_skips_unavailable_options_and_wraps() {
        let mut draft = view_draft();
        let expected = [
            CatalogFormFocus::Schema,
            CatalogFormFocus::Owner,
            CatalogFormFocus::Comment,
            CatalogFormFocus::OutputColumns,
            CatalogFormFocus::Query,
            CatalogFormFocus::SecurityBarrier,
            CatalogFormFocus::CheckOption,
            CatalogFormFocus::Review,
            CatalogFormFocus::Cancel,
            CatalogFormFocus::Name,
        ];
        for focus in expected {
            draft.move_field(1);
            assert_eq!(draft.focus, focus);
        }
        for focus in expected.into_iter().rev().skip(1) {
            draft.move_field(-1);
            assert_eq!(draft.focus, focus);
        }
        draft.move_field(-1);
        assert_eq!(draft.focus, CatalogFormFocus::Name);
        assert!(!draft.focus(CatalogFormFocus::SecurityInvoker));
        assert_eq!(draft.focus, CatalogFormFocus::Name);
    }

    #[test]
    fn catalog_form_focus_materialized_view_order_is_mode_aware_and_wraps() {
        let mut create = materialized_view_draft(true);
        for focus in MATERIALIZED_VIEW_FOCUS_ORDER.iter().copied().skip(1) {
            create.move_field(1, true);
            assert_eq!(create.focus, focus);
        }
        create.move_field(1, true);
        assert_eq!(create.focus, CatalogFormFocus::Name);
        for focus in MATERIALIZED_VIEW_FOCUS_ORDER.iter().copied().rev() {
            create.move_field(-1, true);
            assert_eq!(create.focus, focus);
        }

        let mut edit = materialized_view_draft(false);
        edit.focus = CatalogFormFocus::Comment;
        edit.move_field(1, false);
        assert_eq!(edit.focus, CatalogFormFocus::Tablespace);
        assert!(!edit.focus(CatalogFormFocus::Query, false));
        assert!(!edit.focus(CatalogFormFocus::WithData, false));
        assert_eq!(edit.focus, CatalogFormFocus::Tablespace);
    }

    #[test]
    fn catalog_form_focus_sequence_order_and_wraps() {
        let mut draft = sequence_draft();
        for focus in SEQUENCE_FOCUS_ORDER.iter().copied().skip(1) {
            draft.move_field(1);
            assert_eq!(draft.focus, focus);
        }
        draft.move_field(1);
        assert_eq!(draft.focus, CatalogFormFocus::Name);
        for focus in SEQUENCE_FOCUS_ORDER.iter().copied().rev() {
            draft.move_field(-1);
            assert_eq!(draft.focus, focus);
        }
    }

    #[test]
    fn catalog_form_focus_owner_detection_covers_all_owner_forms() {
        let mut editor = CatalogEditorState {
            database_kind: None,
            draft: Some(CatalogDraft::Schema(SchemaDraft {
                name: TextInput::default(),
                owner: TextInput::default(),
                comment: TextInput::default(),
                selected_field: SCHEMA_OWNER_FIELD,
            })),
            ..CatalogEditorState::default()
        };
        assert!(editor.owner_field_focused());

        let mut view = view_draft();
        view.focus = CatalogFormFocus::Owner;
        editor.draft = Some(CatalogDraft::View(view));
        assert!(editor.owner_field_focused());

        let mut materialized_view = materialized_view_draft(true);
        materialized_view.focus = CatalogFormFocus::Owner;
        editor.draft = Some(CatalogDraft::MaterializedView(materialized_view));
        assert!(editor.owner_field_focused());

        let mut sequence = sequence_draft();
        sequence.focus = CatalogFormFocus::Owner;
        editor.draft = Some(CatalogDraft::Sequence(sequence));
        assert!(editor.owner_field_focused());
    }

    #[test]
    fn view_option_cycles_cover_every_value_and_reject_unavailable_options() {
        let mut draft = view_draft();

        draft.focus = CatalogFormFocus::SecurityBarrier;
        for expected in [Some(true), Some(false), None] {
            assert!(draft.cycle_focused_choice(1));
            assert_eq!(draft.security_barrier.value, expected);
        }

        draft.security_invoker = ViewOption::available(None);
        draft.focus = CatalogFormFocus::SecurityInvoker;
        assert!(draft.cycle_focused_choice(-1));
        assert_eq!(draft.security_invoker.value, Some(false));

        draft.focus = CatalogFormFocus::CheckOption;
        for expected in [Some("LOCAL"), Some("CASCADED"), None] {
            assert!(draft.cycle_focused_choice(1));
            assert_eq!(draft.check_option.value.as_deref(), expected);
        }

        draft.security_invoker = ViewOption::unavailable("not supported");
        draft.focus = CatalogFormFocus::SecurityInvoker;
        assert!(!draft.cycle_focused_choice(1));
        assert_eq!(
            draft.security_invoker,
            ViewOption::unavailable("not supported")
        );
    }

    #[test]
    fn sequence_control_cycles_bounds_and_toggles_cycle() {
        let mut draft = sequence_draft();
        draft.focus = CatalogFormFocus::MinValue;
        for expected in [
            SequenceBound::NoLimit,
            SequenceBound::Value(String::new()),
            SequenceBound::Unset,
        ] {
            assert!(draft.cycle_focused_choice(1));
            assert_eq!(draft.min_value.to_bound(), expected);
        }

        draft.focus = CatalogFormFocus::MaxValue;
        assert!(draft.cycle_focused_choice(-1));
        assert_eq!(
            draft.max_value.to_bound(),
            SequenceBound::Value(String::new())
        );

        draft.focus = CatalogFormFocus::Cycle;
        assert!(draft.toggle_focused());
        assert!(draft.cycle);
        assert!(!draft.cycle_focused_choice(1));
    }

    #[test]
    fn sequence_control_custom_bound_uses_unicode_safe_text_editing() {
        let mut draft = sequence_draft();
        draft.focus = CatalogFormFocus::MinValue;
        draft.min_value = SequenceBound::Value("界-100".into()).into();

        draft.move_home();
        draft.move_right();
        draft.delete();
        draft.insert('9');
        draft.move_end();
        draft.backspace();

        assert_eq!(
            draft.min_value.to_bound(),
            SequenceBound::Value("界910".into())
        );
        assert_eq!(draft.max_value.to_bound(), SequenceBound::Unset);

        for focus in [
            CatalogFormFocus::MaxValue,
            CatalogFormFocus::Cycle,
            CatalogFormFocus::Review,
            CatalogFormFocus::Cancel,
        ] {
            draft.focus = focus;
            draft.insert('7');
            draft.backspace();
        }
        assert_eq!(draft.max_value.to_bound(), SequenceBound::Unset);
    }
}
