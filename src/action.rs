use uuid::Uuid;

use crate::db::{
    ServerInfo,
    catalog::{CatalogKind, CatalogNode},
    query::QueryOutcome,
};
use crate::{
    model::{
        profile_manager::{ProfileField, ProfileInput, ProfileSubmission},
        workspace::{ConnectionIdentity, Focus},
    },
    profile::ConnectionProfile,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewConsole,
    CloseActiveTab,
    NextTab,
    PreviousTab,
    ActivateTab(usize),
    FocusNext,
    FocusPrevious,
    Focus(Focus),
    ShowHelp,
    DismissOverlay,
    OpenProfileManager,
    CloseProfileManager,
    ProfileMove(isize),
    ProfileStartNew,
    ProfileStartEdit,
    ProfileRequestDelete,
    ProfileConfirmDelete,
    ProfileCancelDelete,
    ProfileConnectSelected,
    ProfileFieldNext,
    ProfileFieldPrevious,
    ProfileFocusField(ProfileField),
    ProfileInsert(ProfileInput),
    ProfilePaste(ProfileInput),
    ProfileBackspace,
    ProfileDeleteCharacter,
    ProfileMoveLeft,
    ProfileMoveRight,
    ProfileMoveHome,
    ProfileMoveEnd,
    ProfileCycle(i8),
    ProfileToggle,
    ProfileToggleField(ProfileField),
    ProfileTest,
    ProfileSave {
        connect: bool,
    },
    ProfileTestSucceeded {
        request_id: u64,
        server: ServerInfo,
    },
    ProfileTestFailed {
        request_id: u64,
        message: String,
    },
    ProfileSaved {
        request_id: u64,
        profile: ConnectionProfile,
        warning: Option<String>,
        connect: bool,
    },
    ProfileSaveFailed {
        request_id: u64,
        message: String,
    },
    ProfileDeleted {
        request_id: u64,
        profile_id: Uuid,
        active_connection: Option<ConnectionIdentity>,
    },
    ProfileDeleteFailed {
        request_id: u64,
        message: String,
    },
    CredentialsRequired {
        profile_id: Uuid,
        generation: u64,
        message: String,
    },
    DisconnectCompleted {
        connection: ConnectionIdentity,
    },
    ReplaceEditor(String),
    InsertCharacter(char),
    InsertNewline,
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveHome,
    MoveEnd,
    EnterNormalMode,
    EnterInsertMode,
    EnterAppendMode,
    OpenLineBelow,
    RunActiveSql,
    CancelActiveQuery,
    RefreshCatalog,
    PreviewSelected,
    DdlSelected,
    RequestConnect(Uuid),
    ConnectionSucceeded {
        profile_id: Uuid,
        generation: u64,
        server: ServerInfo,
    },
    ConnectionFailed {
        profile_id: Uuid,
        generation: u64,
        message: String,
    },
    CatalogLoaded {
        profile_id: Uuid,
        generation: u64,
        nodes: Vec<CatalogNode>,
    },
    CatalogFailed {
        profile_id: Uuid,
        generation: u64,
        message: String,
    },
    QueryFinished {
        tab_id: Uuid,
        generation: u64,
        outcome: QueryOutcome,
    },
    QueryFailed {
        tab_id: Uuid,
        generation: u64,
        message: String,
    },
    PreviewFinished {
        tab_id: Uuid,
        generation: u64,
        sql: String,
        outcome: QueryOutcome,
    },
    DdlLoaded {
        tab_id: Uuid,
        generation: u64,
        ddl: String,
    },
    ExplorerMove(isize),
    ExplorerSelect(usize),
    ExplorerToggle,
    GridMove {
        rows: isize,
        columns: isize,
    },
    GridSelect {
        row: usize,
        column: usize,
    },
    ToggleResultView,
    Quit,
}

#[derive(Clone, Debug)]
pub enum Command {
    TestProfile {
        request_id: u64,
        submission: ProfileSubmission,
    },
    SaveProfile {
        request_id: u64,
        submission: ProfileSubmission,
        connect: bool,
    },
    DeleteProfile {
        request_id: u64,
        profile_id: Uuid,
    },
    Disconnect {
        connection: ConnectionIdentity,
    },
    Connect {
        profile_id: Uuid,
        generation: u64,
    },
    LoadCatalog {
        profile_id: Uuid,
        generation: u64,
    },
    RunQuery {
        connection: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        sql: String,
    },
    PreviewTable {
        connection: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        schema: String,
        name: String,
    },
    LoadDdl {
        connection: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        kind: CatalogKind,
        schema: String,
        name: String,
    },
    CancelQuery {
        tab_id: Uuid,
        generation: u64,
    },
    PersistWorkspace,
    Quit,
}
