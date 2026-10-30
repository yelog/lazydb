use crate::model::text_input::{TextInput, TextInputEdit};
use crate::{
    app::App,
    model::{
        editor::EditorMode,
        profile_manager::ProfileManagerPage,
        relation::RelationView,
        relation_edit::RelationGridMode,
        sql_editor_list::SqlEditorListMode,
        tab::{ResultView, WorkspaceTab},
        workspace::{ExplorerSearchPhase, Focus, Overlay},
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ShortcutContext {
    Explorer,
    ExplorerFindEditing,
    ExplorerFindConfirmed,
    ExplorerCatalogSearchEditing,
    ExplorerCatalogSearchConfirmed,
    EditorNormal,
    EditorInsert,
    EditorVisual,
    SqlResultsData,
    SqlOutput,
    RedisKeys,
    RedisKeysFindEditing,
    RedisKeysFindConfirmed,
    RedisPreview,
    RedisPreviewVisual,
    RedisPreviewTable,
    Dashboard,
    MongoDashboard,
    RelationDataBrowse,
    RelationDataEdit,
    RelationDataVisual,
    RelationDataBusy,
    RelationDdl,
    /// DDL-only view for a database user or role (no relation data view).
    PrincipalDdl,
    RecordView,
    DataQueryInput,
    ProfileManagerForm,
    CatalogEditorPicker,
    CatalogEditorForm,
    CatalogEditorTableColumns,
    CatalogEditorTableActions,
    CatalogEditorColumnDetails,
    CatalogEditorPreview,
    CatalogEditorBusy,
    ProfileManagerScope,
    ProfileManagerDelete,
    ConsoleManager,
    ConsoleManagerSearch,
    ConsoleManagerRename,
    ConsoleManagerDeleteConfirm,
    Help,
    ProfileAccess,
    ProfileGroup,
    Message,
    WorkspaceSaveFailure,
    SubstituteConfirmation,
    ExecutionConfirmation,
    ManualCancelConfirmation,
    TransactionExitConfirmation,
    TransactionReviewConfirmation,
    ClearTransactionOutcomeConfirmation,
    TargetSelector,
    DatabaseSelector,
    DeleteConsoleConfirmation,
    PageSizeSelector,
    CatalogDropConfirmation,
    NotificationHistory,
}

#[cfg(test)]
const ALL_SHORTCUT_CONTEXTS: &[ShortcutContext] = &[
    ShortcutContext::Explorer,
    ShortcutContext::ExplorerFindEditing,
    ShortcutContext::ExplorerFindConfirmed,
    ShortcutContext::ExplorerCatalogSearchEditing,
    ShortcutContext::ExplorerCatalogSearchConfirmed,
    ShortcutContext::EditorNormal,
    ShortcutContext::EditorInsert,
    ShortcutContext::EditorVisual,
    ShortcutContext::SqlResultsData,
    ShortcutContext::SqlOutput,
    ShortcutContext::RedisKeys,
    ShortcutContext::RedisKeysFindEditing,
    ShortcutContext::RedisKeysFindConfirmed,
    ShortcutContext::RedisPreview,
    ShortcutContext::RedisPreviewVisual,
    ShortcutContext::RedisPreviewTable,
    ShortcutContext::Dashboard,
    ShortcutContext::MongoDashboard,
    ShortcutContext::RelationDataBrowse,
    ShortcutContext::RelationDataEdit,
    ShortcutContext::RelationDataVisual,
    ShortcutContext::RelationDataBusy,
    ShortcutContext::RelationDdl,
    ShortcutContext::PrincipalDdl,
    ShortcutContext::RecordView,
    ShortcutContext::DataQueryInput,
    ShortcutContext::ProfileManagerForm,
    ShortcutContext::CatalogEditorPicker,
    ShortcutContext::CatalogEditorForm,
    ShortcutContext::CatalogEditorTableColumns,
    ShortcutContext::CatalogEditorTableActions,
    ShortcutContext::CatalogEditorColumnDetails,
    ShortcutContext::CatalogEditorPreview,
    ShortcutContext::CatalogEditorBusy,
    ShortcutContext::ProfileManagerScope,
    ShortcutContext::ProfileManagerDelete,
    ShortcutContext::ConsoleManager,
    ShortcutContext::ConsoleManagerSearch,
    ShortcutContext::ConsoleManagerRename,
    ShortcutContext::ConsoleManagerDeleteConfirm,
    ShortcutContext::Help,
    ShortcutContext::ProfileAccess,
    ShortcutContext::Message,
    ShortcutContext::WorkspaceSaveFailure,
    ShortcutContext::SubstituteConfirmation,
    ShortcutContext::ExecutionConfirmation,
    ShortcutContext::ManualCancelConfirmation,
    ShortcutContext::TransactionExitConfirmation,
    ShortcutContext::TransactionReviewConfirmation,
    ShortcutContext::ClearTransactionOutcomeConfirmation,
    ShortcutContext::TargetSelector,
    ShortcutContext::DatabaseSelector,
    ShortcutContext::DeleteConsoleConfirmation,
    ShortcutContext::PageSizeSelector,
    ShortcutContext::CatalogDropConfirmation,
    ShortcutContext::NotificationHistory,
];

pub(crate) fn shortcut_context(app: &App) -> ShortcutContext {
    shortcut_context_with_overlay(app, true)
}

fn shortcut_context_with_overlay(app: &App, include_help: bool) -> ShortcutContext {
    if let Some(overlay) = app.overlay.as_ref() {
        if !include_help && matches!(overlay, Overlay::Help(_)) {
            // Validate against the pane beneath Help.
        } else {
            return match overlay {
                Overlay::Help(_) => ShortcutContext::Help,
                Overlay::RecordView(_) => ShortcutContext::RecordView,
                Overlay::TextDetail(_) => ShortcutContext::Message,
                Overlay::SqlHistory(_) => ShortcutContext::Message,
                Overlay::ProfileManager => {
                    match app.profile_manager.as_ref().map(|state| state.page) {
                        Some(ProfileManagerPage::Scope) => ShortcutContext::ProfileManagerScope,
                        Some(ProfileManagerPage::ConfirmDelete) => {
                            ShortcutContext::ProfileManagerDelete
                        }
                        Some(ProfileManagerPage::Form) | None => {
                            ShortcutContext::ProfileManagerForm
                        }
                    }
                }
                Overlay::CatalogEditor => match app.catalog_editor.as_ref() {
                    Some(editor) if editor.is_busy() => ShortcutContext::CatalogEditorBusy,
                    Some(editor) => match editor.page {
                        crate::model::catalog_editor::CatalogEditorPage::ObjectPicker => {
                            ShortcutContext::CatalogEditorPicker
                        }
                        crate::model::catalog_editor::CatalogEditorPage::SqlPreview => {
                            ShortcutContext::CatalogEditorPreview
                        }
                        crate::model::catalog_editor::CatalogEditorPage::Form => {
                            match editor.draft.as_ref() {
                                Some(crate::model::catalog_editor::CatalogDraft::Table(draft)) => {
                                    if draft.column_editor.is_some() {
                                        ShortcutContext::CatalogEditorColumnDetails
                                    } else {
                                        match draft.focus {
                                        crate::model::catalog_editor::TableEditorFocus::Columns => {
                                            ShortcutContext::CatalogEditorTableColumns
                                        }
                                        crate::model::catalog_editor::TableEditorFocus::Action(_) => {
                                            ShortcutContext::CatalogEditorTableActions
                                        }
                                        _ => ShortcutContext::CatalogEditorForm,
                                    }
                                    }
                                }
                                _ => ShortcutContext::CatalogEditorForm,
                            }
                        }
                        _ => ShortcutContext::CatalogEditorForm,
                    },
                    None => ShortcutContext::CatalogEditorBusy,
                },
                Overlay::RedisObjectEditor(_) => ShortcutContext::Message,
                Overlay::RedisTableEditor(_) => ShortcutContext::Message,
                Overlay::RedisTableDeleteConfirm(_) => ShortcutContext::DeleteConsoleConfirmation,
                Overlay::RedisValueSaveConfirm { .. } => ShortcutContext::Message,
                Overlay::RedisUnsavedValueConfirm { .. } => ShortcutContext::Message,
                Overlay::SqlEditorList(list) => match list.mode {
                    SqlEditorListMode::Browse => ShortcutContext::ConsoleManager,
                    SqlEditorListMode::Search => ShortcutContext::ConsoleManagerSearch,
                    SqlEditorListMode::Rename { .. } => ShortcutContext::ConsoleManagerRename,
                    SqlEditorListMode::DeleteConfirm { .. } => {
                        ShortcutContext::ConsoleManagerDeleteConfirm
                    }
                },
                Overlay::ProfileAccess { .. } => ShortcutContext::ProfileAccess,
                Overlay::ProfileGroup(_) => ShortcutContext::ProfileGroup,
                Overlay::ExplorerAdd(_) => ShortcutContext::Explorer,
                Overlay::Message { .. } => ShortcutContext::Message,
                Overlay::WorkspaceSaveFailed { .. } => ShortcutContext::WorkspaceSaveFailure,
                Overlay::SubstituteConfirm { .. } => ShortcutContext::SubstituteConfirmation,
                Overlay::ExecutionConfirm { .. } => ShortcutContext::ExecutionConfirmation,
                Overlay::PrincipalMutationConfirm { .. } => ShortcutContext::ExecutionConfirmation,
                Overlay::PrincipalMutationForm(_) => ShortcutContext::ExecutionConfirmation,
                Overlay::ManualCancelConfirm { .. } => ShortcutContext::ManualCancelConfirmation,
                Overlay::TransactionExitConfirm { .. } => {
                    ShortcutContext::TransactionExitConfirmation
                }
                Overlay::RelationTransactionConfirm(_) => {
                    ShortcutContext::TransactionReviewConfirmation
                }
                Overlay::ClearTransactionOutcome { .. } => {
                    ShortcutContext::ClearTransactionOutcomeConfirmation
                }
                Overlay::TransactionMenu { .. } => ShortcutContext::Message,
                Overlay::TargetSelector { .. } => ShortcutContext::TargetSelector,
                Overlay::DatabaseSelector(_) => ShortcutContext::DatabaseSelector,
                Overlay::DeleteConsole { .. } => ShortcutContext::DeleteConsoleConfirmation,
                Overlay::RedisDeleteConfirm { .. } => ShortcutContext::DeleteConsoleConfirmation,
                Overlay::RedisDeletePreparing { .. } => ShortcutContext::DeleteConsoleConfirmation,
                Overlay::MongoManagement(_) => ShortcutContext::CatalogEditorPreview,
                Overlay::MongoPrincipalMutation(_) | Overlay::MongoPrincipalConfirmation { .. } => {
                    ShortcutContext::ExecutionConfirmation
                }
                Overlay::PageSizeSelector { .. } => ShortcutContext::PageSizeSelector,
                Overlay::RedisPreviewFormat { .. } => ShortcutContext::PageSizeSelector,
                Overlay::CatalogDropConfirm { .. } => ShortcutContext::CatalogDropConfirmation,
                Overlay::PrincipalDropConfirm { .. } => ShortcutContext::CatalogDropConfirmation,
                Overlay::CatalogEditorDestructiveConfirm { .. } => {
                    ShortcutContext::CatalogEditorPreview
                }
                Overlay::CatalogEditorDiscardConfirm { .. } => ShortcutContext::CatalogEditorForm,
                Overlay::NotificationHistory(_) => ShortcutContext::NotificationHistory,
                Overlay::NotificationDetail(_) => ShortcutContext::NotificationHistory,
                Overlay::Update(_) => ShortcutContext::Message,
            };
        }
    }
    if app.focus == Focus::Explorer {
        if let Some(find) = app.explorer.find.as_ref() {
            return match find.phase {
                ExplorerSearchPhase::Editing => ShortcutContext::ExplorerFindEditing,
                ExplorerSearchPhase::Confirmed => ShortcutContext::ExplorerFindConfirmed,
            };
        }
        if let Some(search) = app.explorer.search.as_ref() {
            return match search.phase {
                ExplorerSearchPhase::Editing => ShortcutContext::ExplorerCatalogSearchEditing,
                ExplorerSearchPhase::Confirmed => ShortcutContext::ExplorerCatalogSearchConfirmed,
            };
        }
    }
    if matches!(app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data && tab.query.focus.is_some()
    ) || matches!(app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data && tab.query.focus.is_some()
    ) {
        return ShortcutContext::DataQueryInput;
    }
    match app.tabs.get(app.active_tab) {
        Some(WorkspaceTab::RedisBrowser(tab)) if app.focus == Focus::Results => match tab.focus {
            crate::model::redis_browser::RedisBrowserFocus::Keys => {
                match tab.find.as_ref().map(|find| find.phase) {
                    Some(crate::model::redis_browser::RedisFindPhase::Editing) => {
                        ShortcutContext::RedisKeysFindEditing
                    }
                    Some(crate::model::redis_browser::RedisFindPhase::Confirmed) => {
                        ShortcutContext::RedisKeysFindConfirmed
                    }
                    None => ShortcutContext::RedisKeys,
                }
            }
            crate::model::redis_browser::RedisBrowserFocus::Preview => {
                if tab.format.view() == crate::value_preview::ValueView::Table {
                    ShortcutContext::RedisPreviewTable
                } else if matches!(
                    app.active_read_only_editor_mode(),
                    Some(EditorMode::VisualChar | EditorMode::VisualLine | EditorMode::VisualBlock)
                ) {
                    ShortcutContext::RedisPreviewVisual
                } else {
                    ShortcutContext::RedisPreview
                }
            }
        },
        Some(WorkspaceTab::Relation(tab))
            if app.focus == Focus::Results && tab.view == RelationView::Ddl =>
        {
            ShortcutContext::RelationDdl
        }
        Some(WorkspaceTab::PrincipalDdl(_)) if app.focus == Focus::Results => {
            ShortcutContext::PrincipalDdl
        }
        Some(WorkspaceTab::Relation(tab)) if app.focus == Focus::Results => {
            match tab.edit.as_ref().map(|edit| &edit.mode) {
                Some(RelationGridMode::EditCell(_)) | Some(RelationGridMode::EditDocument(_)) => {
                    ShortcutContext::RelationDataEdit
                }
                Some(RelationGridMode::VisualLine { .. }) => ShortcutContext::RelationDataVisual,
                Some(RelationGridMode::Busy) => ShortcutContext::RelationDataBusy,
                Some(RelationGridMode::Browse) | None => ShortcutContext::RelationDataBrowse,
            }
        }
        Some(WorkspaceTab::Sql(tab)) if app.focus == Focus::Results => match tab.result_view {
            ResultView::Data => ShortcutContext::SqlResultsData,
            ResultView::Output | ResultView::Plan => ShortcutContext::SqlOutput,
        },
        Some(WorkspaceTab::Dashboard(_)) if app.focus == Focus::Results => {
            ShortcutContext::Dashboard
        }
        Some(WorkspaceTab::MongoDashboard(_)) if app.focus == Focus::Results => {
            ShortcutContext::MongoDashboard
        }
        _ => match app.focus {
            Focus::Explorer => ShortcutContext::Explorer,
            Focus::Results => ShortcutContext::SqlResultsData,
            Focus::Editor => match app.active_editor_mode() {
                EditorMode::Normal => ShortcutContext::EditorNormal,
                EditorMode::Insert | EditorMode::Replace => ShortcutContext::EditorInsert,
                EditorMode::VisualChar | EditorMode::VisualLine | EditorMode::VisualBlock => {
                    ShortcutContext::EditorVisual
                }
            },
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HelpShortcutId {
    Help,
    OpenOmni,
    Quit,
    TerminalSelection,
    FocusExplorer,
    FocusExplorerLeader,
    FocusResults,
    FocusResultsFromL,
    FocusEditorFromK,
    FocusEditorFromL,
    CyclePaneFocus,
    TogglePaneMaximized,
    ResizeHeightIncrease,
    ResizeHeightDecrease,
    ResizeWidthIncrease,
    ResizeWidthDecrease,
    SmartResizeLeft,
    SmartResizeDown,
    SmartResizeUp,
    SmartResizeRight,
    ResetPaneSizes,
    PreviousTab,
    NextTab,
    PreviousTabAlias,
    NextTabAlias,
    MoveTabRight,
    MoveTabLeft,
    OpenDashboard,
    OpenSqlHistory,
    DashboardToggleView,
    DashboardRefresh,
    MongoMonitorRefresh,
    MongoPrincipalsRefresh,
    DashboardTogglePolling,
    RunSql,
    RunAllSql,
    CloseTab,
    CloseOtherTabs,
    DeleteConsole,
    OpenSqlEditors,
    OpenNotificationHistory,
    OpenNotificationHistoryLeader,
    OpenUpdateCenter,
    ExplorerMoveDown,
    ExplorerMoveUp,
    ExplorerFirst,
    ExplorerLast,
    ExplorerViewTop,
    ExplorerViewMiddle,
    ExplorerViewBottom,
    ExplorerHalfPageDown,
    ExplorerHalfPageUp,
    ExplorerPageDown,
    ExplorerPageUp,
    ExplorerAlignMiddle,
    ExplorerAlignTop,
    ExplorerAlignBottom,
    ExplorerExpand,
    ExplorerCollapse,
    ExplorerToggle,
    ExplorerActivate,
    ExplorerAddToConnection,
    ExplorerEditProfile,
    ExplorerCreateCatalog,
    ExplorerEditCatalog,
    ExplorerCreateGroup,
    ExplorerEditGroup,
    ExplorerMoveToGroup,
    ExplorerDeleteGroup,
    ExplorerDeleteProfile,
    ExplorerConnect,
    ExplorerDisconnect,
    ExplorerRefresh,
    ExplorerPreview,
    ExplorerDdl,
    ExplorerAccess,
    EditorInsert,
    EditorNormal,
    EditorUndo,
    EditorRedo,
    EditorJumpList,
    EditorRun,
    EditorRunInsert,
    EditorFormat,
    EditorIndent,
    EditorCopyStatement,
    EditorCopyBuffer,
    ToggleTransaction,
    TransactionControl,
    OpenTargetSelector,
    OpenDatabaseSelector,
    ResultsMoveLeft,
    ResultsMoveDown,
    ResultsMoveUp,
    ResultsMoveRight,
    ResultsFirstPage,
    ResultsPreviousPage,
    ResultsNextPage,
    ResultsLastPage,
    ResultsPageSize,
    ResultsFirstColumn,
    ResultsLastColumn,
    ResultsFirstRow,
    ResultsLastRow,
    ResultsViewTop,
    ResultsViewMiddle,
    ResultsViewBottom,
    ResultsHalfPageDown,
    ResultsHalfPageUp,
    ResultsPageDown,
    ResultsPageUp,
    ResultsAlignMiddle,
    ResultsAlignTop,
    ResultsAlignBottom,
    ResultsOpenRecordView,
    ResultsCopyCell,
    RelationCopyCell,
    ResultsCopyRow,
    ResultsCopyRowWithHeaders,
    ResultsToggleView,
    RelationWhere,
    RelationOrderBy,
    RelationApplyInputs,
    RelationResizeLeft,
    RelationResizeRight,
    RelationResetWidth,
    RelationRefresh,
    ExplorerFindEdit,
    ExplorerFindConfirm,
    ExplorerFindNext,
    ExplorerFindPrevious,
    ExplorerSearchEdit,
    ExplorerSearchLocate,
    ExplorerSearchNext,
    ExplorerSearchPrevious,
    OutputMove,
    OutputEnds,
    OutputSearch,
    OutputSelect,
    OutputCopy,
    RelationEditApply,
    RelationEditCancel,
    RelationVisualMove,
    RelationVisualYank,
    RelationVisualDelete,
    RelationVisualCancel,
    RelationDdlMove,
    RelationDdlEnds,
    RelationDdlSearch,
    RelationDdlSelect,
    RelationDdlCopy,
    RelationDdlData,
    RecordMoveFields,
    RecordMoveRows,
    RecordEnds,
    RecordClose,
    RecordFirstField,
    DataQueryEdit,
    DataQuerySubmit,
    DataQueryCancel,
    DataQuerySwitch,
    ProfileFormMove,
    ProfileFormActivate,
    ProfileFormSave,
    ProfileFormClose,
    ProfileScopeMove,
    ProfileScopeToggle,
    ProfileScopeRefresh,
    ProfileScopeBack,
    ProfileDeleteConfirm,
    ProfileDeleteCancel,
    ConsoleManagerMove,
    ConsoleManagerSearchMove,
    ConsoleManagerActivate,
    ConsoleManagerCreate,
    ConsoleManagerDelete,
    ConsoleManagerRename,
    ConsoleManagerSearch,
    ConsoleManagerClose,
    ConsoleManagerEdit,
    ConsoleManagerCommit,
    ConsoleManagerCancel,
    ConsoleManagerConfirmDelete,
    ConsoleManagerDeleteCancel,
    HelpEdit,
    HelpMove,
    HelpExecute,
    HelpClose,
    ProfileAccessMove,
    ProfileAccessConfirm,
    ProfileAccessClose,
    MessageClose,
    WorkspaceSaveMove,
    WorkspaceSaveActivate,
    WorkspaceSaveRetry,
    WorkspaceSaveQuit,
    WorkspaceSaveScroll,
    SubstituteChoices,
    SubstituteClose,
    ExecutionConfirm,
    ExecutionCancel,
    ExecutionToggle,
    ExecutionPreviewScroll,
    ManualCancelConfirm,
    ManualCancelKeep,
    ManualCancelToggle,
    TransactionChoices,
    TransactionCancel,
    TransactionToggle,
    TransactionReviewMove,
    TransactionReviewCopy,
    TransactionReviewToggle,
    ClearOutcomeConfirm,
    ClearOutcomeCancel,
    ClearOutcomeToggle,
    TargetMove,
    TargetConfirm,
    TargetCancel,
    DatabaseMove,
    DatabaseConfirm,
    DatabaseCancel,
    DeleteConsoleConfirm,
    DeleteConsoleCancel,
    PageSizeMove,
    PageSizeConfirm,
    PageSizeCancel,
    CatalogDropEdit,
    CatalogDropConfirm,
    CatalogDropCancel,
    CatalogEditorMove,
    CatalogEditorSelect,
    CatalogEditorActivate,
    CatalogEditorEdit,
    CatalogEditorFormMove,
    CatalogEditorPreview,
    CatalogEditorApply,
    CatalogEditorBack,
    CatalogEditorCancel,
    CatalogEditorTableColumnsMove,
    CatalogEditorTableColumnAdd,
    CatalogEditorTableColumnEdit,
    CatalogEditorColumnDetailsMove,
    CatalogEditorColumnDetailsEdit,
    CatalogEditorColumnDetailsToggle,
    CatalogEditorColumnDetailsConfirm,
    CatalogEditorColumnDetailsCancel,
    CatalogEditorTableActionActivate,
    CatalogEditorBusyCancel,
    EditorComplete,
    EditorDeleteWord,
    RelationEditCell,
    RelationInsertRow,
    RelationVisualLine,
    RelationPaste,
    RelationCommit,
    RelationUndo,
    RelationRedo,
    RelationRollback,
    RelationEditText,
    RelationEditBoolean,
    RelationEditSetNull,
    RelationEditRestoreDefault,
    RelationEditUseValue,
    RelationEditTemporal,
    RelationEditJson,
    RelationEditDeleteWord,
    EditorYank,
    RedisPreviewFormat,
    RedisPreviewWrap,
    RedisPreviewNextPage,
    RedisPreviewMove,
    RedisPreviewPage,
    RedisPreviewCopy,
    RedisPreviewSearch,
    RedisKeysMove,
    RedisKeysFind,
    RedisKeysExpand,
    RedisKeysCollapse,
    RedisKeysToggle,
    RedisKeysOpen,
    RedisKeysCopy,
    RedisKeysCreate,
    RedisKeysEdit,
    RedisKeysDelete,
    RedisKeysRefresh,
    RedisKeysPage,
    RedisPreviewTableMove,
    RedisPreviewTableCopy,
    RedisPreviewTableDetails,
    RedisFindConfirm,
    RedisFindCancel,
    RedisFindNext,
    RedisFindPrevious,
    ExplorerFindOpen,
    ExplorerSearchOpen,
    DataQueryWhere,
    DataQueryOrderBy,
    DataQueryCompletionNext,
    DataQueryCompletionPrevious,
    RelationDeleteRow,
    RelationBusyData,
    RelationBusyRefresh,
}

const fn footer_priority(id: HelpShortcutId) -> Option<u8> {
    use HelpShortcutId::*;
    Some(match id {
        ExplorerMoveDown
        | EditorInsert
        | ResultsMoveLeft
        | OutputMove
        | RelationDdlMove
        | RecordMoveFields
        | DataQueryEdit
        | ExplorerFindEdit
        | ExplorerFindNext
        | ExplorerSearchEdit
        | ExplorerSearchNext
        | ProfileFormMove
        | ProfileScopeMove
        | ProfileDeleteConfirm
        | ConsoleManagerMove
        | ConsoleManagerSearchMove
        | ConsoleManagerEdit
        | HelpEdit
        | ProfileAccessMove
        | MessageClose
        | WorkspaceSaveMove
        | WorkspaceSaveActivate
        | WorkspaceSaveRetry
        | WorkspaceSaveQuit
        | WorkspaceSaveScroll
        | SubstituteChoices
        | ExecutionConfirm
        | ManualCancelConfirm
        | TransactionChoices
        | TransactionReviewMove
        | TransactionReviewCopy
        | TransactionReviewToggle
        | ClearOutcomeConfirm
        | TargetMove
        | DatabaseMove
        | DeleteConsoleConfirm
        | RelationEditApply
        | RelationVisualMove
        | PageSizeMove
        | CatalogDropEdit
        | CatalogEditorMove
        | RedisKeysMove
        | RedisPreviewMove
        | RedisPreviewTableMove
        | RedisFindConfirm
        | RedisFindNext => 1,
        ExplorerMoveUp
        | EditorRun
        | ResultsMoveDown
        | OutputEnds
        | RelationDdlEnds
        | RecordMoveRows
        | DataQuerySubmit
        | ExplorerFindConfirm
        | ExplorerFindPrevious
        | ExplorerSearchLocate
        | ExplorerSearchPrevious
        | ProfileFormActivate
        | ProfileScopeToggle
        | ProfileDeleteCancel
        | ConsoleManagerActivate
        | ConsoleManagerCommit
        | ConsoleManagerCancel
        | ConsoleManagerConfirmDelete
        | ConsoleManagerDeleteCancel
        | ConsoleManagerDelete
        | ConsoleManagerRename
        | ConsoleManagerSearch
        | HelpMove
        | ProfileAccessConfirm
        | SubstituteClose
        | ExecutionCancel
        | ManualCancelKeep
        | TransactionCancel
        | ClearOutcomeCancel
        | TargetConfirm
        | DatabaseConfirm
        | DeleteConsoleCancel
        | RelationEditCancel
        | RelationVisualYank
        | PageSizeConfirm
        | PageSizeCancel
        | CatalogDropConfirm
        | CatalogDropCancel
        | CatalogEditorActivate
        | CatalogEditorPreview
        | CatalogEditorApply
        | CatalogEditorBack
        | CatalogEditorColumnDetailsToggle
        | CatalogEditorColumnDetailsConfirm
        | CatalogEditorColumnDetailsCancel
        | CatalogEditorTableActionActivate
        | CatalogEditorBusyCancel
        | CatalogEditorCancel
        | CatalogEditorColumnDetailsMove
        | RelationRedo
        | RelationRollback
        | RedisKeysOpen
        | RedisKeysExpand
        | RedisKeysCollapse
        | RedisKeysToggle
        | RedisPreviewPage
        | RedisPreviewTableCopy
        | RedisFindCancel
        | RedisFindPrevious => 2,
        ExplorerCollapse
        | EditorFormat
        | ResultsMoveUp
        | OutputSearch
        | RelationDdlSearch
        | RecordEnds
        | DataQueryCancel
        | ProfileFormSave
        | ProfileScopeRefresh
        | ConsoleManagerCreate
        | HelpExecute
        | ProfileAccessClose
        | ExecutionToggle
        | ManualCancelToggle
        | TransactionToggle
        | ClearOutcomeToggle
        | TargetCancel
        | DatabaseCancel
        | RelationEditText
        | RelationVisualDelete
        | RedisKeysFind
        | RedisPreviewSearch
        | RedisPreviewFormat
        | RedisPreviewWrap
        | RedisPreviewNextPage
        | RedisKeysCopy
        | RedisKeysCreate
        | RedisKeysEdit
        | RedisKeysDelete
        | RedisKeysRefresh
        | RedisKeysPage
        | RedisPreviewTableDetails => 3,
        RelationEditSetNull | RelationEditRestoreDefault | RelationEditUseValue => 3,
        ExplorerExpand
        | EditorCopyStatement
        | ResultsMoveRight
        | OutputSelect
        | RelationDdlSelect
        | RecordClose
        | DataQuerySwitch
        | ProfileFormClose
        | ProfileScopeBack
        | ConsoleManagerClose
        | HelpClose
        | RelationEditDeleteWord
        | RelationVisualCancel => 4,
        ExplorerActivate
        | EditorNormal
        | ResultsOpenRecordView
        | OutputCopy
        | RelationDdlCopy
        | RelationEditCell => 5,
        ExplorerFindOpen | EditorComplete | ResultsCopyCell | RelationCopyCell
        | RelationDdlData | RelationInsertRow => 6,
        ExplorerSearchOpen | EditorDeleteWord | ResultsCopyRow | RelationVisualLine => 7,
        ResultsToggleView | RelationPaste | ExplorerRefresh | RelationBusyData => 8,
        DataQueryWhere | RelationWhere | RelationCommit => 9,
        DataQueryOrderBy | RelationOrderBy | RelationBusyRefresh => 10,
        Help => 11,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutPrefix {
    Leader,
    EditorLeader,
    EditorLeaderTransaction,
    Window,
    WindowCount(u32),
    Goto,
    GridAlign,
    ExplorerAlign,
    Previous,
    Next,
    RelationDelete,
    RecordViewGoto,
}

impl ShortcutPrefix {
    #[cfg(test)]
    const fn display(self) -> &'static str {
        match self {
            Self::Leader => "Space ",
            Self::EditorLeader => "Space ",
            Self::EditorLeaderTransaction => "Space t",
            Self::Window => "Ctrl-w ",
            Self::WindowCount(_) => "",
            Self::Goto => "g",
            Self::GridAlign => "z",
            Self::ExplorerAlign => "z",
            Self::Previous => "[",
            Self::Next => "]",
            Self::RelationDelete => "d",
            Self::RecordViewGoto => "g",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortcutRequirement {
    Always,
    RelationData,
    PaneDirection(char),
    SqlPaneDirection(char),
    RelationPaneDirection(char),
    PaneResize(char),
    DataQueryAvailable,
    RecordViewAvailable,
    RelationEditAvailable,
    ProfileScopeReady,
    ActiveSqlConsole,
    ExplorerAddAvailable,
    ProfileEditAvailable,
    CatalogCreateAvailable,
    CatalogEditAvailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shortcut {
    pub id: HelpShortcutId,
    pub contexts: &'static [ShortcutContext],
    pub sequence: &'static str,
    pub description: &'static str,
    pub footer_priority: Option<u8>,
    pub prefix: Option<ShortcutPrefix>,
    pub suffix: Option<&'static str>,
    requirement: ShortcutRequirement,
    executable: bool,
}

macro_rules! row {
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None, requirement: ShortcutRequirement::Always, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, display) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None, requirement: ShortcutRequirement::Always, executable: false }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, $prefix:ident, $suffix:literal) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::$prefix), suffix: Some($suffix),
            requirement: ShortcutRequirement::Always, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, relation) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None, requirement: ShortcutRequirement::RelationData, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, $requirement:ident, display) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None, requirement: ShortcutRequirement::$requirement, executable: false }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, $requirement:ident, $prefix:ident, $suffix:literal) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::$prefix), suffix: Some($suffix),
            requirement: ShortcutRequirement::$requirement, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, $requirement:ident, executable) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None, requirement: ShortcutRequirement::$requirement, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, Window, $suffix:literal, pane) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::Window), suffix: Some($suffix),
            requirement: ShortcutRequirement::PaneDirection($suffix.as_bytes()[0] as char), executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, Window, $suffix:literal, sql_pane) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::Window), suffix: Some($suffix),
            requirement: ShortcutRequirement::SqlPaneDirection($suffix.as_bytes()[0] as char), executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, Window, $suffix:literal, relation_pane) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::Window), suffix: Some($suffix),
            requirement: ShortcutRequirement::RelationPaneDirection($suffix.as_bytes()[0] as char), executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, Window, $suffix:literal, always) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::Window), suffix: Some($suffix),
            requirement: ShortcutRequirement::Always, executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, Window, $suffix:literal, resize) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: Some(ShortcutPrefix::Window), suffix: Some($suffix),
            requirement: ShortcutRequirement::PaneResize($suffix.as_bytes()[0] as char), executable: true }
    };
    ($id:ident, [$($context:ident),+], $sequence:literal, $description:literal, smart_resize) => {
        Shortcut { id: HelpShortcutId::$id, contexts: &[$(ShortcutContext::$context),+],
            sequence: $sequence, description: $description, footer_priority: footer_priority(HelpShortcutId::$id),
            prefix: None, suffix: None,
            requirement: ShortcutRequirement::Always, executable: true }
    };
}

static SHORTCUT_CATALOG: &[Shortcut] = &[
    row!(
        Help,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataEdit,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            RedisKeys,
            RedisKeysFindEditing,
            RedisKeysFindConfirmed,
            RedisPreview,
            RedisPreviewVisual,
            RedisPreviewTable,
            Dashboard
        ],
        "? (also F1)",
        "open this help panel",
        display
    ),
    row!(
        OpenOmni,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataEdit,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            RedisKeys,
            RedisKeysFindEditing,
            RedisKeysFindConfirmed,
            RedisPreview,
            RedisPreviewVisual,
            RedisPreviewTable,
            Dashboard
        ],
        "F2",
        "open Omni search"
    ),
    row!(
        Quit,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataEdit,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            RedisKeys,
            RedisKeysFindEditing,
            RedisKeysFindConfirmed,
            RedisPreview,
            RedisPreviewVisual,
            RedisPreviewTable,
            Dashboard
        ],
        "Ctrl-c",
        "quit LazyDB"
    ),
    row!(
        TerminalSelection,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataEdit,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            Dashboard
        ],
        "Ctrl-Shift-s",
        "release mouse for terminal-native selection"
    ),
    row!(
        FocusExplorer,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataVisual,
            RelationDdl,
            PrincipalDdl,
            Dashboard
        ],
        "Ctrl-w h",
        "move focus left to Explorer",
        Window,
        "h",
        pane
    ),
    row!(
        FocusResults,
        [EditorNormal, EditorVisual],
        "Ctrl-w j",
        "move focus down to Results",
        Window,
        "j",
        pane
    ),
    row!(
        FocusResultsFromL,
        [Explorer],
        "Ctrl-w l",
        "move focus right to Results",
        Window,
        "l",
        relation_pane
    ),
    row!(
        FocusEditorFromK,
        [SqlResultsData, SqlOutput],
        "Ctrl-w k",
        "move focus up to Editor",
        Window,
        "k",
        pane
    ),
    row!(
        FocusEditorFromL,
        [Explorer],
        "Ctrl-w l",
        "move focus right to Editor",
        Window,
        "l",
        sql_pane
    ),
    row!(
        CyclePaneFocus,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            Dashboard
        ],
        "Ctrl-w Ctrl-w",
        "cycle pane focus clockwise",
        Window,
        "Ctrl-w",
        always
    ),
    row!(
        TogglePaneMaximized,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            Dashboard,
            RelationDataBrowse,
            RelationDataVisual,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w f",
        "maximize or restore focused pane",
        Window,
        "f",
        always
    ),
    row!(
        PreviousTab,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse
        ],
        "gT",
        "previous tab",
        Goto,
        "T"
    ),
    row!(
        NextTab,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse
        ],
        "gt",
        "next tab",
        Goto,
        "t"
    ),
    row!(
        MoveTabRight,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-Shift-n",
        "move tab right"
    ),
    row!(
        MoveTabLeft,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-Shift-p",
        "move tab left"
    ),
    row!(
        PreviousTabAlias,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "[t",
        "previous tab",
        Previous,
        "t"
    ),
    row!(
        NextTabAlias,
        [
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "]t",
        "next tab",
        Next,
        "t"
    ),
    row!(
        OpenSqlEditors,
        [
            Explorer,
            EditorNormal,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Space s",
        "open console manager",
        Leader,
        "s"
    ),
    row!(
        OpenDashboard,
        [
            Explorer,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl,
            Dashboard
        ],
        "Space b",
        "open database dashboard",
        Leader,
        "b"
    ),
    row!(
        DashboardToggleView,
        [Dashboard],
        "o",
        "toggle overview/process list"
    ),
    row!(DashboardRefresh, [Dashboard], "r", "refresh metrics"),
    row!(
        MongoMonitorRefresh,
        [MongoDashboard],
        "r",
        "refresh MongoDB monitoring"
    ),
    row!(
        MongoPrincipalsRefresh,
        [MongoDashboard],
        "u",
        "list users and roles"
    ),
    row!(
        DashboardTogglePolling,
        [Dashboard],
        "p",
        "toggle automatic refresh"
    ),
    row!(
        CloseTab,
        [
            Explorer,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Space q",
        "close current tab",
        Leader,
        "q"
    ),
    row!(
        CloseOtherTabs,
        [
            Explorer,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl+Shift+q",
        "close other tabs"
    ),
    row!(
        DeleteConsole,
        [Explorer, SqlResultsData, SqlOutput],
        "Space x",
        "permanently delete console",
        Leader,
        "x"
    ),
    row!(
        OpenNotificationHistory,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            DataQueryInput,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl,
            NotificationHistory
        ],
        "F8",
        "open notification history"
    ),
    row!(
        OpenSqlHistory,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            DataQueryInput,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl,
            NotificationHistory
        ],
        "F7",
        "open SQL execution history"
    ),
    row!(
        OpenNotificationHistoryLeader,
        [
            Explorer,
            EditorNormal,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Space m",
        "open notification history",
        Leader,
        "m"
    ),
    row!(
        OpenUpdateCenter,
        [
            Explorer,
            EditorNormal,
            EditorInsert,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDataEdit,
            RelationDataVisual,
            RelationDataBusy,
            RelationDdl,
            PrincipalDdl,
            Dashboard
        ],
        "F9",
        "open update center"
    ),
    row!(ExplorerMoveDown, [Explorer], "j", "move selection down"),
    row!(ExplorerMoveUp, [Explorer], "k", "move selection up"),
    row!(
        ExplorerFirst,
        [Explorer],
        "gg",
        "select first node",
        Goto,
        "g"
    ),
    row!(ExplorerLast, [Explorer], "G", "select last node"),
    row!(ExplorerViewTop, [Explorer], "H", "select top visible node"),
    row!(
        ExplorerViewMiddle,
        [Explorer],
        "M",
        "select middle visible node"
    ),
    row!(
        ExplorerViewBottom,
        [Explorer],
        "L",
        "select bottom visible node"
    ),
    row!(
        ExplorerHalfPageDown,
        [Explorer],
        "Ctrl-d",
        "move down half page"
    ),
    row!(
        ExplorerHalfPageUp,
        [Explorer],
        "Ctrl-u",
        "move up half page"
    ),
    row!(
        ExplorerPageDown,
        [Explorer],
        "Ctrl-f",
        "scroll down one screen"
    ),
    row!(ExplorerPageUp, [Explorer], "Ctrl-b", "scroll up one screen"),
    row!(
        ExplorerAlignMiddle,
        [Explorer],
        "zz",
        "align selection middle",
        ExplorerAlign,
        "z"
    ),
    row!(
        ExplorerAlignTop,
        [Explorer],
        "zt",
        "align selection top",
        ExplorerAlign,
        "t"
    ),
    row!(
        ExplorerAlignBottom,
        [Explorer],
        "zb",
        "align selection bottom",
        ExplorerAlign,
        "b"
    ),
    row!(ExplorerExpand, [Explorer], "l", "expand selection"),
    row!(ExplorerCollapse, [Explorer], "h", "collapse selection"),
    row!(ExplorerToggle, [Explorer], "o", "toggle expand / collapse"),
    row!(
        ExplorerActivate,
        [Explorer],
        "Enter",
        "open table preview / activate"
    ),
    row!(
        ExplorerAddToConnection,
        [Explorer],
        "a",
        "add to connection",
        ExplorerAddAvailable,
        executable
    ),
    row!(
        ExplorerEditProfile,
        [Explorer],
        "e",
        "edit connection",
        ProfileEditAvailable,
        executable
    ),
    row!(
        ExplorerCreateCatalog,
        [Explorer],
        "a",
        "add object",
        CatalogCreateAvailable,
        executable
    ),
    row!(
        ExplorerEditCatalog,
        [Explorer],
        "e",
        "edit selected object",
        CatalogEditAvailable,
        executable
    ),
    row!(
        ExplorerCreateGroup,
        [Explorer],
        "a",
        "new connection group (on a connection or group)"
    ),
    row!(
        ExplorerEditGroup,
        [Explorer],
        "e",
        "edit selected connection group"
    ),
    row!(
        ExplorerMoveToGroup,
        [Explorer],
        "gm",
        "move connection to group",
        Goto,
        "m"
    ),
    row!(
        ExplorerDeleteGroup,
        [Explorer],
        "d",
        "delete selected connection group"
    ),
    row!(ExplorerDeleteProfile, [Explorer], "d", "delete connection"),
    row!(ExplorerConnect, [Explorer], "c", "connect"),
    row!(ExplorerDisconnect, [Explorer], "x", "disconnect"),
    row!(
        ExplorerRefresh,
        [Explorer],
        "r",
        "refresh connection or catalog"
    ),
    row!(ExplorerPreview, [Explorer], "p", "open table preview"),
    row!(ExplorerDdl, [Explorer], "D", "open object DDL"),
    row!(ExplorerAccess, [Explorer], "s", "connection access"),
    row!(
        ExplorerFindOpen,
        [Explorer],
        "/",
        "find visible nodes",
        display
    ),
    row!(
        ExplorerSearchOpen,
        [Explorer],
        "f",
        "search catalog",
        display
    ),
    row!(EditorInsert, [EditorNormal], "i", "enter Insert mode"),
    row!(
        EditorNormal,
        [EditorInsert, EditorVisual],
        "Esc",
        "return to Normal mode"
    ),
    row!(EditorUndo, [EditorNormal], "u", "undo"),
    row!(EditorRedo, [EditorNormal], "Ctrl-r", "redo"),
    row!(
        EditorJumpList,
        [EditorNormal],
        "Ctrl-o / Ctrl-i (Tab)",
        "jump to the previous / next saved editor position"
    ),
    row!(
        EditorRun,
        [EditorNormal, EditorVisual],
        "R / F5",
        "execute current or selected SQL"
    ),
    row!(EditorRunInsert, [EditorInsert], "F5", "execute SQL"),
    row!(
        EditorFormat,
        [EditorNormal, EditorVisual],
        "Space f",
        "format selected / current SQL",
        EditorLeader,
        "f"
    ),
    row!(
        EditorIndent,
        [EditorNormal, EditorVisual],
        ">> / <<; > / <",
        "indent or unindent the current or selected lines"
    ),
    row!(
        EditorCopyStatement,
        [EditorNormal, EditorVisual],
        "Space y",
        "copy current SQL statement or selection",
        EditorLeader,
        "y"
    ),
    row!(
        EditorCopyBuffer,
        [EditorNormal],
        "Space Y",
        "copy complete SQL buffer",
        EditorLeader,
        "Y"
    ),
    row!(
        ToggleTransaction,
        [EditorNormal],
        "Space tt",
        "toggle AUTO / MANUAL transaction",
        EditorLeader,
        "tt"
    ),
    row!(
        TransactionControl,
        [EditorNormal, SqlResultsData],
        "Space tc",
        "commit or roll back transaction",
        EditorLeader,
        "tc"
    ),
    row!(
        OpenTargetSelector,
        [EditorNormal],
        "Space d",
        "choose and reconnect editor target",
        EditorLeader,
        "d"
    ),
    row!(
        OpenDatabaseSelector,
        [
            Explorer,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl,
            EditorNormal
        ],
        "Space D",
        "choose and reconnect workspace database",
        EditorLeader,
        "D"
    ),
    row!(
        FocusExplorerLeader,
        [
            Explorer,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Space c",
        "focus Explorer",
        Leader,
        "c"
    ),
    row!(
        RunSql,
        [Explorer, SqlResultsData, SqlOutput],
        "Space r",
        "run SQL",
        ActiveSqlConsole,
        Leader,
        "r"
    ),
    row!(
        RunAllSql,
        [Explorer, SqlResultsData, SqlOutput],
        "Space R",
        "run all SQL",
        ActiveSqlConsole,
        Leader,
        "R"
    ),
    row!(
        EditorComplete,
        [EditorInsert],
        "Ctrl-Space",
        "trigger completion",
        display
    ),
    row!(
        EditorDeleteWord,
        [EditorInsert],
        "Ctrl-w",
        "delete previous word",
        display
    ),
    row!(EditorYank, [EditorVisual], "y", "copy selection", display),
    row!(
        RedisPreviewFormat,
        [RedisPreview, RedisPreviewVisual],
        "Space f",
        "cycle Redis Preview format",
        Leader,
        "f"
    ),
    row!(
        RedisPreviewWrap,
        [RedisPreview, RedisPreviewVisual],
        "Space w",
        "toggle Redis Preview wrapping",
        Leader,
        "w"
    ),
    row!(
        RedisPreviewNextPage,
        [RedisPreview, RedisPreviewVisual],
        "Space l",
        "load the next Redis Preview page",
        Leader,
        "l"
    ),
    row!(
        RedisPreviewMove,
        [RedisPreview, RedisPreviewVisual],
        "hjkl",
        "move through Preview text"
    ),
    row!(
        RedisPreviewPage,
        [RedisPreview, RedisPreviewVisual],
        "Ctrl-d / Ctrl-u",
        "move by half a page",
        display
    ),
    row!(
        RedisPreviewCopy,
        [RedisPreview, RedisPreviewVisual],
        "yy / y{motion}",
        "copy Preview text",
        display
    ),
    row!(
        RedisPreviewSearch,
        [RedisPreview, RedisPreviewVisual],
        "/ / n / N",
        "search Preview text",
        display
    ),
    row!(
        RedisKeysMove,
        [RedisKeys],
        "hjkl",
        "move through Redis keys"
    ),
    row!(RedisKeysFind, [RedisKeys], "/", "find Redis keys", display),
    row!(
        RedisKeysExpand,
        [RedisKeys, RedisKeysFindConfirmed],
        "Right / l",
        "expand the selected Redis key group",
        display
    ),
    row!(
        RedisKeysCollapse,
        [RedisKeys, RedisKeysFindConfirmed],
        "Left / h",
        "collapse the selected Redis key group",
        display
    ),
    row!(
        RedisKeysToggle,
        [RedisKeys, RedisKeysFindConfirmed],
        "o",
        "toggle the selected Redis key group",
        display
    ),
    row!(
        RedisKeysOpen,
        [RedisKeys, RedisKeysFindConfirmed],
        "Enter",
        "open the selected Redis key or toggle its group",
        display
    ),
    row!(
        RedisKeysCopy,
        [RedisKeys, RedisKeysFindConfirmed],
        "y",
        "copy the selected Redis key",
        display
    ),
    row!(
        RedisKeysCreate,
        [RedisKeys],
        "a",
        "create a Redis object",
        display
    ),
    row!(
        RedisKeysEdit,
        [RedisKeys],
        "e",
        "edit the selected Redis key",
        display
    ),
    row!(
        RedisKeysDelete,
        [RedisKeys],
        "d",
        "delete the selected Redis key",
        display
    ),
    row!(
        RedisKeysRefresh,
        [RedisKeys],
        "r",
        "refresh the Redis key scan",
        display
    ),
    row!(
        RedisKeysPage,
        [RedisKeys, RedisKeysFindConfirmed],
        "PageUp / PageDown",
        "scroll the Redis key tree",
        display
    ),
    row!(
        RedisFindConfirm,
        [RedisKeysFindEditing],
        "Enter",
        "confirm the Redis key search",
        display
    ),
    row!(
        RedisFindCancel,
        [RedisKeysFindEditing, RedisKeysFindConfirmed],
        "Esc",
        "close Redis key search",
        display
    ),
    row!(
        RedisFindNext,
        [RedisKeysFindConfirmed],
        "n",
        "go to the next Redis key match",
        display
    ),
    row!(
        RedisFindPrevious,
        [RedisKeysFindConfirmed],
        "N",
        "go to the previous Redis key match",
        display
    ),
    row!(
        RedisPreviewTableMove,
        [RedisPreviewTable],
        "hjkl / arrows",
        "move through the Redis value table",
        display
    ),
    row!(
        RedisPreviewTableCopy,
        [RedisPreviewTable],
        "y",
        "copy the selected Redis table value",
        display
    ),
    row!(
        RedisPreviewTableDetails,
        [RedisPreviewTable],
        "v",
        "open Redis table value details",
        display
    ),
    row!(
        ResultsMoveLeft,
        [SqlResultsData, RelationDataBrowse],
        "h",
        "move through cells left"
    ),
    row!(
        ResultsMoveDown,
        [SqlResultsData, RelationDataBrowse],
        "j",
        "move through cells down"
    ),
    row!(
        ResultsMoveUp,
        [SqlResultsData, RelationDataBrowse],
        "k",
        "move through cells up"
    ),
    row!(
        ResultsMoveRight,
        [SqlResultsData, RelationDataBrowse],
        "l",
        "move through cells right"
    ),
    row!(
        ResultsFirstPage,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-Home",
        "load first data page"
    ),
    row!(
        ResultsPreviousPage,
        [SqlResultsData, RelationDataBrowse],
        "PageUp",
        "load previous data page"
    ),
    row!(
        ResultsNextPage,
        [SqlResultsData, RelationDataBrowse],
        "PageDown",
        "load next data page"
    ),
    row!(
        ResultsLastPage,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-End",
        "load last data page"
    ),
    row!(
        ResultsPageSize,
        [SqlResultsData, RelationDataBrowse],
        "P",
        "change data page size"
    ),
    row!(
        ResultsFirstColumn,
        [SqlResultsData, RelationDataBrowse],
        "0/^",
        "select first column"
    ),
    row!(
        ResultsLastColumn,
        [SqlResultsData, RelationDataBrowse],
        "$",
        "select last column"
    ),
    row!(
        ResultsFirstRow,
        [SqlResultsData, RelationDataBrowse],
        "gg",
        "select first row",
        Goto,
        "g"
    ),
    row!(
        ResultsLastRow,
        [SqlResultsData, RelationDataBrowse],
        "G",
        "select last row"
    ),
    row!(
        ResultsViewTop,
        [SqlResultsData, RelationDataBrowse],
        "H",
        "select top visible row"
    ),
    row!(
        ResultsViewMiddle,
        [SqlResultsData, RelationDataBrowse],
        "M",
        "select middle visible row"
    ),
    row!(
        ResultsViewBottom,
        [SqlResultsData, RelationDataBrowse],
        "L",
        "select bottom visible row"
    ),
    row!(
        ResultsHalfPageDown,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-d",
        "move down half a page"
    ),
    row!(
        ResultsHalfPageUp,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-u",
        "move up half a page"
    ),
    row!(
        ResultsPageDown,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-f",
        "scroll down one screen within the current data page"
    ),
    row!(
        ResultsPageUp,
        [SqlResultsData, RelationDataBrowse],
        "Ctrl-b",
        "scroll up one screen within the current data page"
    ),
    row!(
        ResultsAlignMiddle,
        [SqlResultsData, RelationDataBrowse],
        "zz",
        "align selected row to middle",
        GridAlign,
        "z"
    ),
    row!(
        ResultsAlignTop,
        [SqlResultsData, RelationDataBrowse],
        "zt",
        "align selected row to top",
        GridAlign,
        "t"
    ),
    row!(
        ResultsAlignBottom,
        [SqlResultsData, RelationDataBrowse],
        "zb",
        "align selected row to bottom",
        GridAlign,
        "b"
    ),
    row!(
        ResultsOpenRecordView,
        [SqlResultsData, RelationDataBrowse],
        "v",
        "open Record View",
        RecordViewAvailable,
        executable
    ),
    row!(ResultsCopyCell, [SqlResultsData], "y", "copy selected cell"),
    row!(
        RelationCopyCell,
        [RelationDataBrowse],
        "y",
        "copy selected cell"
    ),
    row!(
        ResultsCopyRow,
        [SqlResultsData, RelationDataBrowse],
        "Y",
        "copy selected row as TSV"
    ),
    row!(
        ResultsCopyRowWithHeaders,
        [SqlResultsData, RelationDataBrowse],
        "Space Y",
        "copy row with headers",
        Leader,
        "Y"
    ),
    row!(
        ResultsToggleView,
        [SqlResultsData, SqlOutput],
        "o",
        "switch Data / Output"
    ),
    row!(
        RelationWhere,
        [RelationDataBrowse],
        "/",
        "focus WHERE filter",
        DataQueryAvailable,
        executable
    ),
    row!(
        RelationOrderBy,
        [RelationDataBrowse],
        "s",
        "focus ORDER BY",
        DataQueryAvailable,
        executable
    ),
    row!(
        RelationApplyInputs,
        [RelationDataBrowse],
        "Enter",
        "apply preview inputs",
        display
    ),
    row!(
        RelationResizeLeft,
        [RelationDataBrowse],
        "[",
        "resize selected column narrower",
        display
    ),
    row!(
        RelationResizeRight,
        [RelationDataBrowse],
        "]",
        "resize selected column wider",
        display
    ),
    row!(
        RelationResetWidth,
        [RelationDataBrowse],
        "=",
        "reset selected column width",
        relation
    ),
    row!(
        RelationRefresh,
        [RelationDataBrowse, RelationDdl, PrincipalDdl],
        "r",
        "refresh relation"
    ),
    row!(
        DataQueryWhere,
        [SqlResultsData],
        "/",
        "focus WHERE filter",
        DataQueryAvailable,
        display
    ),
    row!(
        DataQueryOrderBy,
        [SqlResultsData],
        "s",
        "focus ORDER BY",
        DataQueryAvailable,
        display
    ),
    row!(
        RelationEditCell,
        [RelationDataBrowse],
        "e",
        "edit selected cell",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationInsertRow,
        [RelationDataBrowse],
        "a",
        "insert row",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationVisualLine,
        [RelationDataBrowse],
        "V",
        "select rows",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationPaste,
        [RelationDataBrowse],
        "p",
        "paste row",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationCommit,
        [RelationDataBrowse],
        "Ctrl-s",
        "review and commit changes",
        RelationEditAvailable,
        executable
    ),
    row!(
        RelationUndo,
        [RelationDataBrowse],
        "u",
        "undo row changes",
        display
    ),
    row!(
        RelationRedo,
        [RelationDataBrowse],
        "Ctrl-r",
        "redo row changes"
    ),
    row!(
        RelationEditText,
        [RelationDataEdit],
        "type / Backspace",
        "edit cell value",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditSetNull,
        [RelationDataEdit],
        "Alt-N",
        "set explicit SQL NULL",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditRestoreDefault,
        [RelationDataEdit],
        "Alt-D",
        "restore DEFAULT (insert drafts only)",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditUseValue,
        [RelationDataEdit],
        "Alt-V",
        "use the current typed value/template",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditBoolean,
        [RelationDataEdit],
        "Left/Right Space t/f",
        "choose boolean",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditTemporal,
        [RelationDataEdit],
        "Left/Right  [/]  type",
        "edit temporal fields; browse month",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditJson,
        [RelationDataEdit],
        "Enter  Ctrl-S  Ctrl-F  Esc",
        "edit, format, apply, or cancel JSON without running SQL",
        RelationEditAvailable,
        display
    ),
    row!(
        RelationEditDeleteWord,
        [RelationDataEdit],
        "Ctrl-w",
        "delete previous word",
        RelationEditAvailable,
        display
    ),
    row!(
        ResizeHeightIncrease,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w +",
        "increase focused pane height",
        Window,
        "+",
        resize
    ),
    row!(
        ResizeHeightDecrease,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w -",
        "decrease focused pane height",
        Window,
        "-",
        resize
    ),
    row!(
        ResizeWidthIncrease,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w >",
        "increase focused pane width",
        Window,
        ">",
        resize
    ),
    row!(
        ResizeWidthDecrease,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w <",
        "decrease focused pane width",
        Window,
        "<",
        resize
    ),
    row!(
        SmartResizeLeft,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Cmd+Ctrl+Shift+h",
        "move visible pane divider left",
        smart_resize
    ),
    row!(
        SmartResizeDown,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Cmd+Ctrl+Shift+j",
        "move visible pane divider down",
        smart_resize
    ),
    row!(
        SmartResizeUp,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Cmd+Ctrl+Shift+k",
        "move visible pane divider up",
        smart_resize
    ),
    row!(
        SmartResizeRight,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Cmd+Ctrl+Shift+l",
        "move visible pane divider right",
        smart_resize
    ),
    row!(
        ResetPaneSizes,
        [
            Explorer,
            EditorNormal,
            EditorVisual,
            SqlResultsData,
            SqlOutput,
            RelationDataBrowse,
            RelationDdl,
            PrincipalDdl
        ],
        "Ctrl-w =",
        "restore default pane sizes",
        Window,
        "="
    ),
    row!(
        ExplorerFindEdit,
        [ExplorerFindEditing],
        "type / Backspace / Ctrl-w / Ctrl-u",
        "edit; Ctrl-w deletes previous word",
        display
    ),
    row!(
        ExplorerFindConfirm,
        [ExplorerFindEditing],
        "Enter",
        "confirm search",
        display
    ),
    row!(
        ExplorerFindNext,
        [ExplorerFindConfirmed],
        "n",
        "next match",
        display
    ),
    row!(
        ExplorerFindPrevious,
        [ExplorerFindConfirmed],
        "N",
        "previous match",
        display
    ),
    row!(
        ExplorerSearchEdit,
        [ExplorerCatalogSearchEditing],
        "type / Backspace / Ctrl-w / Ctrl-u",
        "edit; Ctrl-w deletes previous word",
        display
    ),
    row!(
        ExplorerSearchLocate,
        [ExplorerCatalogSearchEditing],
        "Enter",
        "locate selected result",
        display
    ),
    row!(
        ExplorerSearchNext,
        [ExplorerCatalogSearchConfirmed],
        "n",
        "next match",
        display
    ),
    row!(
        ExplorerSearchPrevious,
        [ExplorerCatalogSearchConfirmed],
        "N",
        "previous match",
        display
    ),
    row!(
        OutputMove,
        [SqlOutput],
        "j/k",
        "move through output",
        display
    ),
    row!(
        OutputEnds,
        [SqlOutput],
        "gg/G",
        "move to output ends",
        display
    ),
    row!(OutputSearch, [SqlOutput], "/", "search output", display),
    row!(OutputSelect, [SqlOutput], "v/V", "select output", display),
    row!(OutputCopy, [SqlOutput], "y", "copy selection", display),
    row!(
        RelationEditApply,
        [RelationDataEdit],
        "Enter",
        "apply cell edit",
        display
    ),
    row!(
        RelationEditCancel,
        [RelationDataEdit],
        "Esc",
        "cancel cell edit",
        display
    ),
    row!(
        RelationVisualMove,
        [RelationDataVisual],
        "j/k",
        "extend selected rows",
        display
    ),
    row!(
        RelationVisualYank,
        [RelationDataVisual],
        "y",
        "yank selected rows",
        display
    ),
    row!(
        RelationVisualDelete,
        [RelationDataVisual],
        "d",
        "delete selected rows",
        display
    ),
    row!(
        RelationVisualCancel,
        [RelationDataVisual],
        "V",
        "cancel row selection",
        display
    ),
    row!(
        RelationDdlMove,
        [RelationDdl, PrincipalDdl],
        "j/k",
        "move through DDL",
        display
    ),
    row!(
        RelationDdlEnds,
        [RelationDdl, PrincipalDdl],
        "gg/G",
        "move to DDL ends",
        display
    ),
    row!(
        RelationDdlSearch,
        [RelationDdl, PrincipalDdl],
        "/",
        "search DDL",
        display
    ),
    row!(
        RelationDdlSelect,
        [RelationDdl, PrincipalDdl],
        "v/V",
        "select DDL",
        display
    ),
    row!(
        RelationDdlCopy,
        [RelationDdl, PrincipalDdl],
        "y",
        "copy selection",
        display
    ),
    row!(
        RelationDdlData,
        [RelationDdl],
        "p",
        "return to Data",
        display
    ),
    row!(
        RelationDeleteRow,
        [RelationDataBrowse],
        "dd",
        "delete row",
        RelationDelete,
        "d"
    ),
    row!(
        RecordFirstField,
        [RecordView],
        "gg",
        "first field",
        RecordViewGoto,
        "g"
    ),
    row!(
        RecordMoveFields,
        [RecordView],
        "j/k",
        "move through fields",
        display
    ),
    row!(
        RecordMoveRows,
        [RecordView],
        "h/l",
        "move through records",
        display
    ),
    row!(
        RecordEnds,
        [RecordView],
        "gg/G",
        "move to first / last field",
        display
    ),
    row!(
        RecordClose,
        [RecordView],
        "Esc/q/v",
        "close Record View",
        display
    ),
    row!(
        DataQueryEdit,
        [DataQueryInput],
        "type / Backspace",
        "edit query input",
        display
    ),
    row!(
        DataQuerySubmit,
        [DataQueryInput],
        "Enter",
        "apply query",
        display
    ),
    row!(
        DataQueryCancel,
        [DataQueryInput],
        "Esc",
        "cancel input",
        display
    ),
    row!(
        DataQuerySwitch,
        [DataQueryInput],
        "Tab/Shift-Tab",
        "switch query input",
        display
    ),
    row!(
        DataQueryCompletionNext,
        [DataQueryInput],
        "Ctrl-n",
        "next completion",
        display
    ),
    row!(
        DataQueryCompletionPrevious,
        [DataQueryInput],
        "Ctrl-p",
        "previous completion",
        display
    ),
    row!(
        ProfileFormMove,
        [ProfileManagerForm],
        "Tab/Shift-Tab",
        "move between fields",
        display
    ),
    row!(
        ProfileFormActivate,
        [ProfileManagerForm],
        "Space",
        "activate field or option",
        display
    ),
    row!(
        ProfileFormSave,
        [ProfileManagerForm],
        "Enter",
        "save and connect",
        display
    ),
    row!(
        ProfileFormClose,
        [ProfileManagerForm],
        "Esc",
        "close profile manager",
        display
    ),
    row!(
        ProfileScopeMove,
        [ProfileManagerScope],
        "j/k",
        "move through scope",
        display
    ),
    row!(
        ProfileScopeToggle,
        [ProfileManagerScope],
        "Space",
        "toggle scope row",
        ProfileScopeReady,
        display
    ),
    row!(
        ProfileScopeRefresh,
        [ProfileManagerScope],
        "r",
        "refresh scope",
        ProfileScopeReady,
        display
    ),
    row!(
        ProfileScopeBack,
        [ProfileManagerScope],
        "Esc/Enter",
        "return to form",
        display
    ),
    row!(
        ProfileDeleteConfirm,
        [ProfileManagerDelete],
        "Enter",
        "delete profile",
        display
    ),
    row!(
        ProfileDeleteCancel,
        [ProfileManagerDelete],
        "Esc",
        "cancel delete",
        display
    ),
    row!(
        ConsoleManagerMove,
        [ConsoleManager],
        "j/k or Up/Down",
        "move selection",
        display
    ),
    row!(
        ConsoleManagerActivate,
        [ConsoleManager, ConsoleManagerSearch],
        "Enter",
        "open or focus console",
        display
    ),
    row!(
        ConsoleManagerSearchMove,
        [ConsoleManagerSearch],
        "Up/Down",
        "move selection",
        display
    ),
    row!(
        ConsoleManagerCreate,
        [ConsoleManager],
        "a",
        "new console",
        display
    ),
    row!(
        ConsoleManagerDelete,
        [ConsoleManager],
        "d",
        "delete console",
        display
    ),
    row!(
        ConsoleManagerConfirmDelete,
        [ConsoleManagerDeleteConfirm],
        "Enter/y",
        "permanently delete console",
        display
    ),
    row!(
        ConsoleManagerRename,
        [ConsoleManager],
        "r",
        "rename console",
        display
    ),
    row!(
        ConsoleManagerSearch,
        [ConsoleManager],
        "/",
        "search consoles",
        display
    ),
    row!(
        ConsoleManagerClose,
        [ConsoleManager],
        "Esc",
        "close or cancel",
        display
    ),
    row!(
        ConsoleManagerEdit,
        [ConsoleManagerSearch, ConsoleManagerRename],
        "type / Backspace / Delete / Ctrl-w / Ctrl-u",
        "edit search or name",
        display
    ),
    row!(
        ConsoleManagerCommit,
        [ConsoleManagerRename],
        "Enter",
        "save new name",
        display
    ),
    row!(
        ConsoleManagerCancel,
        [ConsoleManagerSearch, ConsoleManagerRename],
        "Esc",
        "cancel or close",
        display
    ),
    row!(
        ConsoleManagerDeleteCancel,
        [ConsoleManagerDeleteConfirm],
        "Esc/n/q",
        "cancel delete",
        display
    ),
    row!(
        HelpEdit,
        [Help],
        "type / Backspace",
        "filter shortcuts",
        display
    ),
    row!(HelpMove, [Help], "Up/Down", "move selection", display),
    row!(
        HelpExecute,
        [Help],
        "Enter",
        "run selected shortcut",
        display
    ),
    row!(HelpClose, [Help], "Esc", "close help", display),
    row!(
        ProfileAccessMove,
        [ProfileAccess],
        "j/k",
        "move selection",
        display
    ),
    row!(
        ProfileAccessConfirm,
        [ProfileAccess],
        "Enter",
        "apply access choice",
        display
    ),
    row!(
        ProfileAccessClose,
        [ProfileAccess],
        "Esc/q",
        "cancel",
        display
    ),
    row!(MessageClose, [Message], "Esc/q", "close message", display),
    row!(
        WorkspaceSaveMove,
        [WorkspaceSaveFailure],
        "Tab/Arrows",
        "move action focus",
        display
    ),
    row!(
        WorkspaceSaveActivate,
        [WorkspaceSaveFailure],
        "Enter",
        "activate action",
        display
    ),
    row!(
        WorkspaceSaveRetry,
        [WorkspaceSaveFailure],
        "r",
        "retry save when available",
        display
    ),
    row!(
        WorkspaceSaveQuit,
        [WorkspaceSaveFailure],
        "d",
        "quit without saving",
        display
    ),
    row!(
        WorkspaceSaveScroll,
        [WorkspaceSaveFailure],
        "PageUp/PageDown",
        "scroll save error details",
        display
    ),
    row!(
        SubstituteChoices,
        [SubstituteConfirmation],
        "y/n/a/l",
        "choose replacement action",
        display
    ),
    row!(
        SubstituteClose,
        [SubstituteConfirmation],
        "Esc/q",
        "stop substitution",
        display
    ),
    row!(
        ExecutionConfirm,
        [ExecutionConfirmation],
        "Enter",
        "activate selected action",
        display
    ),
    row!(
        ExecutionCancel,
        [ExecutionConfirmation],
        "Esc/n/q",
        "cancel execution",
        display
    ),
    row!(
        ExecutionToggle,
        [ExecutionConfirmation],
        "Tab/Shift-Tab/Left/Right",
        "change choice",
        display
    ),
    row!(
        ExecutionPreviewScroll,
        [ExecutionConfirmation],
        "Up/Down/PageUp/PageDown",
        "scroll read-only SQL preview",
        display
    ),
    row!(
        ManualCancelConfirm,
        [ManualCancelConfirmation],
        "Enter/c",
        "cancel query and roll back",
        display
    ),
    row!(
        ManualCancelKeep,
        [ManualCancelConfirmation],
        "Esc/k",
        "keep query running",
        display
    ),
    row!(
        ManualCancelToggle,
        [ManualCancelConfirmation],
        "Tab/Left/Right",
        "change choice",
        display
    ),
    row!(
        TransactionChoices,
        [TransactionExitConfirmation],
        "a/r/c/Enter",
        "choose transaction outcome",
        display
    ),
    row!(
        TransactionCancel,
        [TransactionExitConfirmation],
        "Esc/n",
        "cancel exit",
        display
    ),
    row!(
        TransactionToggle,
        [TransactionExitConfirmation],
        "Tab/Left/Right",
        "change choice",
        display
    ),
    row!(
        TransactionReviewMove,
        [TransactionReviewConfirmation],
        "hjkl / gg / G / PageUp/PageDown",
        "navigate SQL preview",
        display
    ),
    row!(
        TransactionReviewCopy,
        [TransactionReviewConfirmation],
        "v/V/y or mouse drag",
        "select and copy SQL",
        display
    ),
    row!(
        TransactionReviewToggle,
        [TransactionReviewConfirmation],
        "Tab/Shift-Tab",
        "focus preview or action",
        display
    ),
    row!(
        ClearOutcomeConfirm,
        [ClearTransactionOutcomeConfirmation],
        "Enter",
        "clear unknown outcome",
        display
    ),
    row!(
        ClearOutcomeCancel,
        [ClearTransactionOutcomeConfirmation],
        "Esc",
        "cancel",
        display
    ),
    row!(
        ClearOutcomeToggle,
        [ClearTransactionOutcomeConfirmation],
        "Tab/Left/Right",
        "change choice",
        display
    ),
    row!(
        DatabaseMove,
        [TargetSelector],
        "j/k",
        "move target selection",
        display
    ),
    row!(
        DatabaseConfirm,
        [TargetSelector],
        "Enter",
        "choose target",
        display
    ),
    row!(TargetCancel, [TargetSelector], "Esc", "cancel", display),
    row!(
        TargetMove,
        [DatabaseSelector],
        "j/k, Up/Down, Tab",
        "move database selection",
        display
    ),
    row!(
        TargetConfirm,
        [DatabaseSelector],
        "Enter",
        "switch database",
        display
    ),
    row!(DatabaseCancel, [DatabaseSelector], "Esc", "cancel", display),
    row!(
        DeleteConsoleConfirm,
        [DeleteConsoleConfirmation],
        "Tab, then Enter",
        "activate focused action",
        display
    ),
    row!(
        DeleteConsoleCancel,
        [DeleteConsoleConfirmation],
        "Esc",
        "cancel delete",
        display
    ),
    row!(
        PageSizeMove,
        [PageSizeSelector],
        "j/k or Up/Down",
        "move page size selection",
        display
    ),
    row!(
        PageSizeConfirm,
        [PageSizeSelector],
        "Enter",
        "apply page size",
        display
    ),
    row!(
        PageSizeCancel,
        [PageSizeSelector],
        "Esc",
        "cancel page size",
        display
    ),
    row!(
        CatalogDropEdit,
        [CatalogDropConfirmation],
        "Tab / Shift-Tab / Left / Right",
        "switch confirmation option",
        display
    ),
    row!(
        CatalogDropConfirm,
        [CatalogDropConfirmation],
        "Enter",
        "activate selected option",
        display
    ),
    row!(
        CatalogDropCancel,
        [CatalogDropConfirmation],
        "Esc",
        "cancel catalog drop",
        display
    ),
    row!(
        CatalogEditorMove,
        [CatalogEditorPicker],
        "j/k",
        "move through object types",
        display
    ),
    row!(
        CatalogEditorSelect,
        [CatalogEditorPicker],
        "Enter",
        "choose object type",
        display
    ),
    row!(
        CatalogEditorActivate,
        [CatalogEditorPicker],
        "Enter",
        "choose object type",
        display
    ),
    row!(
        CatalogEditorEdit,
        [CatalogEditorForm],
        "type / Backspace",
        "edit object name",
        display
    ),
    row!(
        CatalogEditorFormMove,
        [CatalogEditorForm],
        "Tab/Shift-Tab/Up/Down",
        "move between fields",
        display
    ),
    row!(
        CatalogEditorPreview,
        [CatalogEditorForm],
        "Enter",
        "preview mutation",
        display
    ),
    row!(
        CatalogEditorApply,
        [CatalogEditorPreview],
        "Enter",
        "apply mutation",
        display
    ),
    row!(
        CatalogEditorBack,
        [CatalogEditorPreview],
        "Esc",
        "return to form",
        display
    ),
    row!(
        CatalogEditorCancel,
        [
            CatalogEditorForm,
            CatalogEditorPicker,
            CatalogEditorTableColumns,
            CatalogEditorTableActions,
            CatalogEditorBusy
        ],
        "Esc",
        "close/cancel editor",
        display
    ),
    row!(
        CatalogEditorTableColumnsMove,
        [CatalogEditorTableColumns],
        "Up/Down",
        "move selected row",
        display
    ),
    row!(
        CatalogEditorTableColumnAdd,
        [CatalogEditorTableColumns],
        "a",
        "add column below",
        display
    ),
    row!(
        CatalogEditorTableColumnEdit,
        [CatalogEditorTableColumns],
        "e",
        "edit selected column",
        display
    ),
    row!(
        CatalogEditorColumnDetailsMove,
        [CatalogEditorColumnDetails],
        "Tab/Shift-Tab/Up/Down",
        "move focus",
        display
    ),
    row!(
        CatalogEditorColumnDetailsEdit,
        [CatalogEditorColumnDetails],
        "type / Backspace",
        "edit text field",
        display
    ),
    row!(
        CatalogEditorColumnDetailsToggle,
        [CatalogEditorColumnDetails],
        "Space",
        "toggle Nullable/Identity",
        display
    ),
    row!(
        CatalogEditorColumnDetailsConfirm,
        [CatalogEditorColumnDetails],
        "Enter",
        "confirm changes",
        display
    ),
    row!(
        CatalogEditorColumnDetailsCancel,
        [CatalogEditorColumnDetails],
        "Esc",
        "cancel changes",
        display
    ),
    row!(
        CatalogEditorTableActionActivate,
        [CatalogEditorTableActions],
        "Enter/Space",
        "activate selected action",
        display
    ),
    row!(
        CatalogEditorBusyCancel,
        [CatalogEditorBusy],
        "Esc",
        "cancel",
        display
    ),
    row!(
        RelationBusyData,
        [RelationDataBusy],
        "p",
        "return to Data",
        display
    ),
    row!(
        RelationBusyRefresh,
        [RelationDataBusy],
        "r",
        "refresh relation",
        display
    ),
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShortcutCapabilities {
    relation_data: bool,
    focus: Focus,
    relation_layout: bool,
    data_query_available: bool,
    relation_edit_available: bool,
    record_view_available: bool,
    profile_scope_loading: bool,
    active_sql_console: bool,
    pub(crate) profile_edit_available: bool,
    pub(crate) explorer_add_available: bool,
    pub(crate) catalog_create_available: bool,
    pub(crate) catalog_edit_available: bool,
}

impl ShortcutCapabilities {
    pub fn relation_data() -> Self {
        Self {
            relation_data: true,
            focus: Focus::Results,
            relation_layout: true,
            data_query_available: true,
            relation_edit_available: true,
            record_view_available: true,
            profile_scope_loading: false,
            active_sql_console: true,
            explorer_add_available: false,
            profile_edit_available: false,
            catalog_create_available: false,
            catalog_edit_available: false,
        }
    }
}

pub(crate) fn shortcut_capabilities(app: &App) -> ShortcutCapabilities {
    let data_query_available = match app.tabs.get(app.active_tab) {
        Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => matches!(
            tab.query.capability,
            crate::model::data_query::DataQueryCapability::Sql
        ),
        Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => matches!(
            tab.query.capability,
            crate::model::data_query::DataQueryCapability::Relation
        ),
        _ => false,
    };
    let relation_edit_available = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(tab))
            if tab.view == RelationView::Data
                && tab.edit.as_ref().is_some_and(|edit| !matches!(edit.mode, RelationGridMode::Busy))
    );
    let (rows, columns) = app.active_grid_dimensions_for_input();
    let (profile_edit_available, catalog_create_available, catalog_edit_available) =
        catalog_editor_capabilities(app);
    ShortcutCapabilities {
        relation_data: matches!(app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data),
        focus: app.focus,
        relation_layout: app.is_active_relation_tab(),
        data_query_available,
        relation_edit_available,
        record_view_available: rows > 0 && columns > 0,
        profile_scope_loading: app
            .profile_manager
            .as_ref()
            .is_some_and(|manager| manager.scope_discovery_loading()),
        active_sql_console: app.active_console_opt().is_some(),
        explorer_add_available: explorer_add_available(app),
        profile_edit_available,
        catalog_create_available,
        catalog_edit_available,
    }
}

fn explorer_add_available(app: &App) -> bool {
    use crate::model::explorer::ExplorerNodeId;

    let Some(selected) = app.explorer.normalized.selected.as_ref() else {
        return false;
    };
    match selected {
        ExplorerNodeId::EmptyProfiles => true,
        ExplorerNodeId::Profile(profile_id) => {
            app.profiles.iter().any(|profile| profile.id == *profile_id)
        }
        ExplorerNodeId::PrincipalGroup { profile_id }
        | ExplorerNodeId::PrincipalNotice { profile_id } => {
            app.profiles.iter().any(|profile| profile.id == *profile_id)
        }
        ExplorerNodeId::Principal { entry } => app
            .profiles
            .iter()
            .any(|profile| profile.id == entry.profile_id),
        _ => false,
    }
}

fn catalog_editor_capabilities(app: &App) -> (bool, bool, bool) {
    use crate::db::catalog_mutation::CatalogMutationAnchor;
    use crate::model::explorer::ExplorerNodeId;

    let Some(selected) = app.explorer.normalized.selected.as_ref() else {
        return (false, false, false);
    };
    let profile_edit_available = matches!(selected, ExplorerNodeId::Profile(_));
    let create = app.selected_catalog_create_options().is_some();
    let edit = {
        let Some(profile_id) = selected.profile_id() else {
            return (profile_edit_available, create, false);
        };
        if !app.profiles.iter().any(|profile| profile.id == profile_id) {
            return (profile_edit_available, create, false);
        }
        let entry = match selected {
            ExplorerNodeId::Catalog(id) => app
                .explorer
                .normalized
                .profiles
                .get(&profile_id)
                .and_then(|state| state.catalog.get(id)),
            _ => None,
        };
        let anchor = match selected {
            ExplorerNodeId::Catalog(id) => CatalogMutationAnchor::Catalog(id.clone()),
            ExplorerNodeId::Profile(id) => CatalogMutationAnchor::Profile { profile_id: *id },
            ExplorerNodeId::Group { parent, group } => CatalogMutationAnchor::Group {
                schema: parent.clone(),
                group: *group,
            },
            _ => return (profile_edit_available, create, false),
        };
        let session = match selected {
            ExplorerNodeId::Catalog(id) => {
                let database = id
                    .native_path
                    .first()
                    .filter(|value| value.as_str() != "__role__")
                    .map(String::as_str);
                app.catalog_edit_session(profile_id, database)
            }
            _ => None,
        };
        matches!(selected, ExplorerNodeId::Catalog(_))
            && session.is_some_and(|session| {
                session
                    .mutation_capabilities
                    .can_edit(&anchor, entry)
                    .unwrap_or(false)
            })
    };
    (profile_edit_available, create, edit)
}

fn available(shortcut: &Shortcut, capabilities: ShortcutCapabilities) -> bool {
    match shortcut.requirement {
        ShortcutRequirement::Always => true,
        ShortcutRequirement::RelationData => capabilities.relation_data,
        ShortcutRequirement::PaneDirection(direction) => match direction {
            'h' => matches!(capabilities.focus, Focus::Editor | Focus::Results),
            'j' => capabilities.focus == Focus::Editor,
            'k' => capabilities.focus == Focus::Results && !capabilities.relation_layout,
            'l' => capabilities.focus == Focus::Explorer,
            _ => false,
        },
        ShortcutRequirement::SqlPaneDirection(direction) => {
            !capabilities.relation_layout
                && direction == 'l'
                && capabilities.focus == Focus::Explorer
        }
        ShortcutRequirement::RelationPaneDirection(direction) => {
            capabilities.relation_layout
                && direction == 'l'
                && capabilities.focus == Focus::Explorer
        }
        ShortcutRequirement::PaneResize(operator) => {
            crate::model::workspace::pane_resize(capabilities.focus, operator, 1).is_some()
        }
        ShortcutRequirement::DataQueryAvailable => capabilities.data_query_available,
        ShortcutRequirement::RecordViewAvailable => capabilities.record_view_available,
        ShortcutRequirement::RelationEditAvailable => capabilities.relation_edit_available,
        ShortcutRequirement::ProfileScopeReady => !capabilities.profile_scope_loading,
        ShortcutRequirement::ActiveSqlConsole => capabilities.active_sql_console,
        ShortcutRequirement::ExplorerAddAvailable => capabilities.explorer_add_available,
        ShortcutRequirement::ProfileEditAvailable => capabilities.profile_edit_available,
        ShortcutRequirement::CatalogCreateAvailable => capabilities.catalog_create_available,
        ShortcutRequirement::CatalogEditAvailable => capabilities.catalog_edit_available,
    }
}

#[cfg(test)]
fn shortcut_catalog() -> &'static [Shortcut] {
    SHORTCUT_CATALOG
}

pub(crate) fn shortcuts(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
) -> Vec<Shortcut> {
    SHORTCUT_CATALOG
        .iter()
        .copied()
        .filter(|shortcut| {
            shortcut.contexts.contains(&context) && available(shortcut, capabilities)
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn filtered_shortcuts(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    query: &str,
) -> Vec<Shortcut> {
    filtered_shortcuts_with_bindings(context, capabilities, query, None)
}

pub(crate) fn filtered_shortcuts_with_bindings(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    query: &str,
    bindings: Option<&crate::config::KeyBindings>,
) -> Vec<Shortcut> {
    let tokens = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    shortcuts(context, capabilities)
        .into_iter()
        .filter(|shortcut| pane_binding_configured(shortcut.id, bindings))
        .filter(|shortcut| {
            let configured = pane_command_for_shortcut(shortcut.id)
                .and_then(|command| bindings.and_then(|bindings| bindings.display_for(command)));
            let sequence = configured.unwrap_or_else(|| shortcut.sequence.to_owned());
            let haystack = format!("{} {}", sequence, shortcut.description).to_lowercase();
            tokens.iter().all(|token| haystack.contains(token))
        })
        .collect()
}

pub(crate) fn pane_command_for_shortcut(id: HelpShortcutId) -> Option<&'static str> {
    match id {
        HelpShortcutId::FocusExplorer => Some("focus-pane-left"),
        HelpShortcutId::FocusResults => Some("focus-pane-down"),
        HelpShortcutId::FocusEditorFromK => Some("focus-pane-up"),
        HelpShortcutId::FocusResultsFromL | HelpShortcutId::FocusEditorFromL => {
            Some("focus-pane-right")
        }
        HelpShortcutId::TogglePaneMaximized => Some("toggle-pane-maximized"),
        HelpShortcutId::ResetPaneSizes => Some("reset-pane-sizes"),
        HelpShortcutId::SmartResizeLeft => Some("smart-resize-pane-left"),
        HelpShortcutId::SmartResizeDown => Some("smart-resize-pane-down"),
        HelpShortcutId::SmartResizeUp => Some("smart-resize-pane-up"),
        HelpShortcutId::SmartResizeRight => Some("smart-resize-pane-right"),
        _ => None,
    }
}

fn pane_binding_configured(
    id: HelpShortcutId,
    bindings: Option<&crate::config::KeyBindings>,
) -> bool {
    pane_command_for_shortcut(id)
        .and_then(|command| bindings.map(|bindings| bindings.configured_for(command)))
        .unwrap_or(true)
}

pub(crate) fn configured_sequence(
    shortcut: &Shortcut,
    bindings: Option<&crate::config::KeyBindings>,
) -> String {
    if let Some(command) = pane_command_for_shortcut(shortcut.id)
        && let Some(bindings) = bindings
    {
        return bindings.display_for(command).unwrap_or_default();
    }
    let command = match shortcut.id {
        HelpShortcutId::Help => Some("help"),
        HelpShortcutId::OpenOmni => Some("omni"),
        HelpShortcutId::TerminalSelection => Some("terminal-selection"),
        HelpShortcutId::OpenDashboard => Some("open-dashboard"),
        HelpShortcutId::OpenSqlHistory => Some("open-sql-history"),
        HelpShortcutId::OpenNotificationHistory => Some("notification-history"),
        HelpShortcutId::FocusExplorerLeader => Some("open-explorer"),
        HelpShortcutId::OpenSqlEditors => Some("open-editors"),
        HelpShortcutId::RunSql => Some("run-leader-statement"),
        HelpShortcutId::RunAllSql => Some("run-leader-buffer"),
        HelpShortcutId::OpenTargetSelector => Some("open-target-selector"),
        HelpShortcutId::OpenDatabaseSelector => Some("open-database-selector"),
        HelpShortcutId::NextTab => Some("next-tab"),
        HelpShortcutId::PreviousTab => Some("previous-tab"),
        HelpShortcutId::MoveTabRight => Some("move-tab-right"),
        HelpShortcutId::MoveTabLeft => Some("move-tab-left"),
        HelpShortcutId::CloseTab => Some("close-tab"),
        HelpShortcutId::ExplorerMoveDown => Some("explorer-move-down"),
        HelpShortcutId::ExplorerMoveUp => Some("explorer-move-up"),
        HelpShortcutId::ExplorerExpand => Some("explorer-expand"),
        HelpShortcutId::ExplorerCollapse => Some("explorer-collapse"),
        HelpShortcutId::ResultsMoveLeft => Some("results-move-left"),
        HelpShortcutId::ResultsMoveDown => Some("results-move-down"),
        HelpShortcutId::ResultsMoveUp => Some("results-move-up"),
        HelpShortcutId::ResultsMoveRight => Some("results-move-right"),
        HelpShortcutId::ResultsFirstPage => Some("results-first-page"),
        HelpShortcutId::ResultsPreviousPage => Some("results-previous-page"),
        HelpShortcutId::ResultsNextPage => Some("results-next-page"),
        HelpShortcutId::ResultsLastPage => Some("results-last-page"),
        HelpShortcutId::ResultsPageSize => Some("results-page-size"),
        HelpShortcutId::ResultsOpenRecordView => Some("results-open-record"),
        HelpShortcutId::ResultsCopyCell => Some("results-copy-cell"),
        HelpShortcutId::ResultsCopyRow => Some("results-copy-row"),
        HelpShortcutId::ResultsCopyRowWithHeaders => Some("results-copy-row-headers"),
        HelpShortcutId::ResultsToggleView => Some("results-toggle-view"),
        HelpShortcutId::ResultsFirstColumn => Some("results-first-column"),
        HelpShortcutId::ResultsLastColumn => Some("results-last-column"),
        HelpShortcutId::ResultsAlignMiddle => Some("results-align-middle"),
        HelpShortcutId::ResultsAlignTop => Some("results-align-top"),
        HelpShortcutId::ResultsAlignBottom => Some("results-align-bottom"),
        HelpShortcutId::ExplorerFindOpen => Some("explorer-find"),
        HelpShortcutId::ExplorerSearchOpen => Some("explorer-search"),
        HelpShortcutId::ExplorerRefresh => Some("explorer-refresh"),
        HelpShortcutId::ExplorerToggle => Some("explorer-toggle"),
        HelpShortcutId::OpenUpdateCenter => Some("update"),
        _ => None,
    };
    command
        .and_then(|command| bindings.and_then(|bindings| bindings.display_for(command)))
        .unwrap_or_else(|| shortcut.sequence.to_owned())
}

#[allow(dead_code)] // Consumed by pending-sequence rendering in Task 7.
pub(crate) fn prefix_shortcuts(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    prefix: ShortcutPrefix,
) -> Vec<Shortcut> {
    prefix_shortcuts_with_bindings(context, capabilities, prefix, None)
}

pub(crate) fn prefix_shortcuts_with_bindings(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    prefix: ShortcutPrefix,
    bindings: Option<&crate::config::KeyBindings>,
) -> Vec<Shortcut> {
    let mut candidates = SHORTCUT_CATALOG
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, shortcut)| {
            shortcut.contexts.contains(&context)
                && (shortcut.prefix == Some(prefix)
                    || context == ShortcutContext::RelationDataBrowse
                        && prefix == ShortcutPrefix::EditorLeader
                        && shortcut.prefix == Some(ShortcutPrefix::Leader)
                    || context == ShortcutContext::EditorNormal
                        && prefix == ShortcutPrefix::EditorLeader
                        && shortcut.id == HelpShortcutId::OpenNotificationHistoryLeader
                    || prefix == ShortcutPrefix::EditorLeaderTransaction
                        && matches!(
                            shortcut.id,
                            HelpShortcutId::ToggleTransaction | HelpShortcutId::TransactionControl
                        )
                    || matches!(prefix, ShortcutPrefix::WindowCount(_))
                        && shortcut.prefix == Some(ShortcutPrefix::Window))
                && !(context == ShortcutContext::EditorNormal
                    && prefix == ShortcutPrefix::Leader
                    && shortcut.id == HelpShortcutId::OpenNotificationHistoryLeader)
                && available(shortcut, capabilities)
                && pane_binding_configured(shortcut.id, bindings)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(index, shortcut)| {
        (prefix_rank(prefix, shortcut.id).unwrap_or(u8::MAX), *index)
    });
    candidates
        .into_iter()
        .map(|(_, mut shortcut)| {
            if prefix == ShortcutPrefix::EditorLeaderTransaction {
                shortcut.prefix = Some(prefix);
                shortcut.suffix = shortcut.suffix.and_then(|suffix| suffix.strip_prefix('t'));
            }
            shortcut
        })
        .collect()
}

#[allow(dead_code)] // Kept with prefix_shortcuts until Task 7 consumes it.
fn prefix_rank(prefix: ShortcutPrefix, id: HelpShortcutId) -> Option<u8> {
    use HelpShortcutId as Id;
    Some(match prefix {
        ShortcutPrefix::Leader => match id {
            Id::OpenDashboard => 1,
            Id::RunSql => 2,
            Id::RunAllSql => 3,
            Id::CloseTab => 4,
            Id::FocusExplorerLeader => 5,
            Id::DeleteConsole => 6,
            Id::OpenSqlEditors => 7,
            Id::EditorFormat => 8,
            Id::EditorCopyStatement => 9,
            Id::EditorCopyBuffer => 10,
            Id::ToggleTransaction => 11,
            Id::TransactionControl => 12,
            Id::OpenTargetSelector => 13,
            Id::ResultsCopyRowWithHeaders => 14,
            _ => return None,
        },
        ShortcutPrefix::EditorLeader => match id {
            Id::EditorFormat => 1,
            Id::EditorCopyStatement => 2,
            Id::EditorCopyBuffer => 3,
            Id::OpenTargetSelector => 4,
            Id::ToggleTransaction => 5,
            Id::TransactionControl => 6,
            Id::CloseTab => 7,
            Id::OpenSqlEditors => 8,
            Id::FocusExplorerLeader => 9,
            Id::OpenNotificationHistoryLeader => 10,
            _ => return None,
        },
        ShortcutPrefix::EditorLeaderTransaction => match id {
            Id::ToggleTransaction => 1,
            Id::TransactionControl => 2,
            _ => return None,
        },
        ShortcutPrefix::Window | ShortcutPrefix::WindowCount(_) => match id {
            Id::FocusExplorer => 1,
            Id::FocusResults | Id::FocusResultsFromL => 2,
            Id::FocusEditorFromK | Id::FocusEditorFromL => 3,
            Id::CyclePaneFocus => 4,
            Id::TogglePaneMaximized => 5,
            Id::ResizeHeightIncrease => 6,
            Id::ResizeHeightDecrease => 7,
            Id::ResizeWidthIncrease => 8,
            Id::ResizeWidthDecrease => 9,
            Id::ResetPaneSizes => 10,
            _ => return None,
        },
        ShortcutPrefix::Goto => match id {
            Id::ExplorerFirst | Id::ResultsFirstRow | Id::RecordFirstField => 1,
            Id::NextTab => 2,
            Id::PreviousTab => 3,
            Id::ExplorerMoveToGroup => 4,
            _ => return None,
        },
        ShortcutPrefix::ExplorerAlign | ShortcutPrefix::GridAlign => match id {
            Id::ExplorerAlignMiddle | Id::ResultsAlignMiddle => 1,
            Id::ExplorerAlignTop | Id::ResultsAlignTop => 2,
            Id::ExplorerAlignBottom | Id::ResultsAlignBottom => 3,
            _ => return None,
        },
        ShortcutPrefix::Previous => match id {
            Id::PreviousTabAlias => 1,
            _ => return None,
        },
        ShortcutPrefix::Next => match id {
            Id::NextTabAlias => 1,
            _ => return None,
        },
        ShortcutPrefix::RelationDelete => match id {
            Id::RelationDeleteRow => 1,
            _ => return None,
        },
        ShortcutPrefix::RecordViewGoto => match id {
            Id::RecordFirstField => 1,
            _ => return None,
        },
    })
}

#[allow(dead_code)] // Consumed by the footer renderer in Task 6.
pub(crate) fn footer_shortcuts(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
) -> Vec<Shortcut> {
    footer_shortcuts_with_bindings(context, capabilities, None)
}

pub(crate) fn footer_shortcuts_with_bindings(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    bindings: Option<&crate::config::KeyBindings>,
) -> Vec<Shortcut> {
    let mut indexed = shortcuts(context, capabilities)
        .into_iter()
        .filter(|shortcut| pane_binding_configured(shortcut.id, bindings))
        .enumerate()
        .filter_map(|(index, mut shortcut)| {
            let priority = footer_rank(context, capabilities, shortcut.id)?;
            shortcut.footer_priority = Some(priority);
            Some((index, shortcut))
        })
        .collect::<Vec<_>>();
    indexed.sort_by_key(|(index, shortcut)| (shortcut.footer_priority.unwrap(), *index));
    indexed.into_iter().map(|(_, shortcut)| shortcut).collect()
}

#[allow(dead_code)] // Kept beside footer_shortcuts until Task 6 consumes it.
fn footer_rank(
    context: ShortcutContext,
    capabilities: ShortcutCapabilities,
    id: HelpShortcutId,
) -> Option<u8> {
    use HelpShortcutId as Id;
    match context {
        ShortcutContext::EditorInsert => match id {
            Id::EditorNormal => Some(1),
            Id::EditorComplete => Some(2),
            Id::EditorDeleteWord => Some(3),
            Id::EditorRunInsert => Some(4),
            Id::Help => Some(5),
            _ => None,
        },
        ShortcutContext::EditorVisual => match id {
            Id::EditorYank => Some(1),
            Id::EditorNormal => Some(2),
            Id::EditorRun => Some(3),
            Id::EditorFormat => Some(4),
            Id::Help => Some(5),
            _ => None,
        },
        ShortcutContext::SqlOutput => match id {
            Id::OutputMove => Some(1),
            Id::OutputEnds => Some(2),
            Id::OutputSearch => Some(3),
            Id::OutputSelect => Some(4),
            Id::OutputCopy => Some(5),
            Id::ResultsToggleView => Some(6),
            Id::Help => Some(7),
            _ => None,
        },
        ShortcutContext::SqlResultsData => match id {
            Id::ResultsMoveLeft => Some(1),
            Id::ResultsMoveDown => Some(2),
            Id::ResultsMoveUp => Some(3),
            Id::ResultsMoveRight => Some(4),
            Id::ResultsOpenRecordView => Some(5),
            Id::ResultsCopyCell => Some(6),
            Id::ResultsCopyRow => Some(7),
            Id::ResultsToggleView => Some(8),
            Id::DataQueryWhere => Some(9),
            Id::Help => Some(10),
            _ => None,
        },
        ShortcutContext::Dashboard => match id {
            Id::DashboardToggleView => Some(1),
            Id::DashboardRefresh => Some(2),
            Id::DashboardTogglePolling => Some(3),
            Id::Help => Some(4),
            _ => None,
        },
        ShortcutContext::RelationDdl => match id {
            Id::RelationDdlMove => Some(1),
            Id::RelationDdlEnds => Some(2),
            Id::RelationDdlSearch => Some(3),
            Id::RelationDdlSelect => Some(4),
            Id::RelationDdlCopy => Some(5),
            Id::RelationDdlData => Some(6),
            Id::RelationRefresh => Some(7),
            Id::Help => Some(8),
            _ => None,
        },
        ShortcutContext::RelationDataBrowse if capabilities.relation_edit_available => match id {
            Id::ResultsMoveLeft => Some(1),
            Id::ResultsMoveDown => Some(2),
            Id::ResultsMoveUp => Some(3),
            Id::ResultsMoveRight => Some(4),
            Id::RelationCopyCell => Some(5),
            Id::RelationEditCell => Some(7),
            Id::RelationInsertRow => Some(8),
            Id::RelationVisualLine => Some(9),
            Id::RelationPaste => Some(10),
            Id::RelationCommit => Some(11),
            _ => None,
        },
        ShortcutContext::RelationDataBrowse => match id {
            Id::ResultsMoveLeft => Some(1),
            Id::ResultsMoveDown => Some(2),
            Id::ResultsMoveUp => Some(3),
            Id::ResultsMoveRight => Some(4),
            Id::ResultsOpenRecordView => Some(5),
            Id::RelationCopyCell => Some(6),
            Id::ResultsCopyRow => Some(8),
            Id::RelationWhere => Some(9),
            Id::RelationOrderBy => Some(10),
            Id::RelationRefresh => Some(11),
            _ => None,
        },
        ShortcutContext::RelationDataEdit => match id {
            Id::RelationEditApply => Some(1),
            Id::RelationEditCancel => Some(2),
            Id::RelationEditText => Some(3),
            Id::RelationEditBoolean => Some(4),
            Id::RelationEditTemporal => Some(5),
            Id::RelationEditSetNull => Some(6),
            Id::RelationEditRestoreDefault => Some(7),
            Id::RelationEditUseValue => Some(8),
            Id::RelationEditDeleteWord => Some(9),
            _ => None,
        },
        ShortcutContext::RelationDataVisual => match id {
            Id::RelationVisualMove => Some(1),
            Id::RelationVisualYank => Some(2),
            Id::RelationVisualDelete => Some(3),
            Id::RelationVisualCancel => Some(4),
            _ => None,
        },
        ShortcutContext::RelationDataBusy => match id {
            Id::RelationBusyData => Some(2),
            Id::RelationBusyRefresh => Some(3),
            Id::Help => Some(4),
            _ => None,
        },
        ShortcutContext::CatalogEditorTableColumns => match id {
            Id::CatalogEditorTableColumnsMove => Some(1),
            Id::CatalogEditorTableColumnAdd => Some(2),
            Id::CatalogEditorTableColumnEdit => Some(3),
            Id::CatalogEditorCancel => Some(4),
            _ => None,
        },
        ShortcutContext::CatalogEditorTableActions => match id {
            Id::CatalogEditorTableColumnsMove => Some(1),
            Id::CatalogEditorTableActionActivate => Some(2),
            Id::CatalogEditorCancel => Some(3),
            _ => None,
        },
        ShortcutContext::CatalogEditorBusy => match id {
            Id::CatalogEditorBusyCancel => Some(1),
            _ => None,
        },
        ShortcutContext::CatalogEditorForm => match id {
            Id::CatalogEditorFormMove => Some(1),
            Id::CatalogEditorEdit => Some(2),
            Id::CatalogEditorPreview => Some(3),
            Id::CatalogEditorCancel => Some(4),
            _ => None,
        },
        ShortcutContext::Message => match id {
            Id::MessageClose => Some(1),
            _ => None,
        },
        ShortcutContext::CatalogEditorColumnDetails => match id {
            Id::CatalogEditorColumnDetailsMove => Some(1),
            Id::CatalogEditorColumnDetailsEdit => Some(2),
            Id::CatalogEditorColumnDetailsToggle => Some(3),
            Id::CatalogEditorColumnDetailsConfirm => Some(4),
            Id::CatalogEditorColumnDetailsCancel => Some(5),
            _ => None,
        },
        _ => footer_priority(id),
    }
}

pub(crate) fn shortcut_is_executable(id: HelpShortcutId) -> bool {
    SHORTCUT_CATALOG
        .iter()
        .find(|shortcut| shortcut.id == id)
        .is_some_and(|shortcut| shortcut.executable)
}

pub(crate) fn shortcut_is_available_in_app(app: &App, id: HelpShortcutId) -> bool {
    let context = shortcut_context_with_overlay(app, false);
    let capabilities = shortcut_capabilities(app);
    SHORTCUT_CATALOG.iter().any(|shortcut| {
        shortcut.id == id
            && shortcut.executable
            && shortcut.contexts.contains(&context)
            && available(shortcut, capabilities)
    })
}

pub(crate) fn context_name(context: ShortcutContext) -> &'static str {
    match context {
        ShortcutContext::Explorer
        | ShortcutContext::ExplorerFindEditing
        | ShortcutContext::ExplorerFindConfirmed
        | ShortcutContext::ExplorerCatalogSearchEditing
        | ShortcutContext::ExplorerCatalogSearchConfirmed => "EXPLORER",
        ShortcutContext::EditorNormal
        | ShortcutContext::EditorInsert
        | ShortcutContext::EditorVisual => "EDITOR",
        ShortcutContext::SqlResultsData
        | ShortcutContext::SqlOutput
        | ShortcutContext::RelationDataBrowse
        | ShortcutContext::RelationDataEdit
        | ShortcutContext::RelationDataVisual
        | ShortcutContext::RelationDataBusy
        | ShortcutContext::RelationDdl
        | ShortcutContext::PrincipalDdl => "RESULTS",
        ShortcutContext::RedisKeys => "REDIS KEYS",
        ShortcutContext::RedisKeysFindEditing | ShortcutContext::RedisKeysFindConfirmed => {
            "REDIS KEYS · FIND"
        }
        ShortcutContext::RedisPreview | ShortcutContext::RedisPreviewVisual => "REDIS VALUE · TEXT",
        ShortcutContext::RedisPreviewTable => "REDIS VALUE · TABLE",
        ShortcutContext::Dashboard => "DASHBOARD",
        ShortcutContext::MongoDashboard => "MONGODB MONITOR",
        ShortcutContext::RecordView => "RECORD VIEW",
        ShortcutContext::DataQueryInput => "DATA QUERY",
        ShortcutContext::ProfileManagerForm
        | ShortcutContext::ProfileManagerScope
        | ShortcutContext::ProfileManagerDelete => "PROFILE MANAGER",
        ShortcutContext::CatalogEditorPicker => "CATALOG PICKER",
        ShortcutContext::CatalogEditorForm => "CATALOG FORM",
        ShortcutContext::CatalogEditorTableColumns => "NEW TABLE COLUMNS",
        ShortcutContext::CatalogEditorTableActions => "TABLE ACTIONS",
        ShortcutContext::CatalogEditorColumnDetails => "COLUMN DETAILS",
        ShortcutContext::CatalogEditorPreview => "CATALOG PREVIEW",
        ShortcutContext::CatalogEditorBusy => "CATALOG BUSY",
        ShortcutContext::ConsoleManager
        | ShortcutContext::ConsoleManagerSearch
        | ShortcutContext::ConsoleManagerRename
        | ShortcutContext::ConsoleManagerDeleteConfirm => "CONSOLE MANAGER",
        ShortcutContext::Help => "HELP",
        ShortcutContext::ProfileAccess => "PROFILE ACCESS",
        ShortcutContext::ProfileGroup => "PROFILE GROUP",
        ShortcutContext::Message => "MESSAGE",
        ShortcutContext::WorkspaceSaveFailure => "WORKSPACE SAVE FAILURE",
        ShortcutContext::SubstituteConfirmation => "SUBSTITUTE",
        ShortcutContext::ExecutionConfirmation => "EXECUTION",
        ShortcutContext::ManualCancelConfirmation => "CANCELLATION",
        ShortcutContext::TransactionExitConfirmation => "TRANSACTION",
        ShortcutContext::TransactionReviewConfirmation => "TRANSACTION REVIEW",
        ShortcutContext::ClearTransactionOutcomeConfirmation => "TRANSACTION OUTCOME",
        ShortcutContext::TargetSelector => "TARGET SELECTOR",
        ShortcutContext::DatabaseSelector => "DATABASE SELECTOR",
        ShortcutContext::DeleteConsoleConfirmation => "DELETE CONSOLE",
        ShortcutContext::PageSizeSelector => "PAGE SIZE",
        ShortcutContext::CatalogDropConfirmation => "CATALOG DROP",
        ShortcutContext::NotificationHistory => "NOTIFICATIONS",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpState {
    pub(crate) context: ShortcutContext,
    pub(crate) capabilities: ShortcutCapabilities,
    pub(crate) query: TextInput,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) viewport_rows: usize,
    pub(crate) bindings: crate::config::KeyBindings,
}

impl HelpState {
    pub fn new(context: ShortcutContext, capabilities: ShortcutCapabilities) -> Self {
        Self {
            context,
            capabilities,
            query: TextInput::default(),
            selected: 0,
            scroll: 0,
            viewport_rows: 0,
            bindings: crate::config::AppConfig::default()
                .keybindings
                .key_bindings()
                .expect("embedded default keybindings must be valid"),
        }
    }

    pub fn with_bindings(
        context: ShortcutContext,
        capabilities: ShortcutCapabilities,
        bindings: crate::config::KeyBindings,
    ) -> Self {
        Self {
            context,
            capabilities,
            query: TextInput::default(),
            selected: 0,
            scroll: 0,
            viewport_rows: 0,
            bindings,
        }
    }
    pub(crate) fn edit(&mut self, edit: TextInputEdit) {
        self.query.apply(edit);
        self.selected = 0;
        self.scroll = 0;
    }
    pub(crate) fn paste(&mut self, value: &str) {
        self.query.paste(
            value
                .chars()
                .map(|character| match character {
                    '\r' | '\n' | '\t' => ' ',
                    c => c,
                })
                .collect::<String>(),
        );
        self.selected = 0;
        self.scroll = 0;
    }
    pub(crate) fn move_selection(&mut self, delta: isize, count: usize) {
        self.query.finish_edit_group();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if delta.is_negative() {
            self.selected
                .checked_sub(delta.unsigned_abs())
                .unwrap_or(count - 1)
        } else {
            (self.selected + delta as usize) % count
        };
        self.ensure_selected_visible(count);
    }

    pub(crate) fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows;
        self.normalize_scroll(self.entries().len());
        self.ensure_selected_visible(self.entries().len());
    }

    pub(crate) fn scroll_by(&mut self, delta: isize) {
        let count = self.entries().len();
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.normalize_scroll(count);
        if count > 0 && self.viewport_rows > 0 {
            self.selected = self.selected.clamp(
                self.scroll,
                (self.scroll + self.viewport_rows - 1).min(count - 1),
            );
        }
    }

    pub(crate) fn set_scroll(&mut self, offset: usize) {
        let count = self.entries().len();
        self.scroll = offset;
        self.normalize_scroll(count);
        if count > 0 && self.viewport_rows > 0 {
            self.selected = self.selected.clamp(
                self.scroll,
                (self.scroll + self.viewport_rows - 1).min(count - 1),
            );
        }
    }

    pub(crate) fn entries(&self) -> Vec<Shortcut> {
        filtered_shortcuts_with_bindings(
            self.context,
            self.capabilities,
            self.query.value(),
            Some(&self.bindings),
        )
    }

    fn normalize_scroll(&mut self, count: usize) {
        if self.viewport_rows > 0 {
            self.scroll = self.scroll.min(count.saturating_sub(self.viewport_rows));
        }
        if count == 0 {
            self.selected = 0;
            self.scroll = 0;
        }
    }

    fn ensure_selected_visible(&mut self, count: usize) {
        if count == 0 || self.viewport_rows == 0 {
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        if self.selected >= self.scroll + self.viewport_rows {
            self.scroll = self.selected.saturating_sub(self.viewport_rows - 1);
        }
        self.normalize_scroll(count);
    }
    pub(crate) fn selected_id(&self) -> Option<HelpShortcutId> {
        filtered_shortcuts_with_bindings(
            self.context,
            self.capabilities,
            self.query.value(),
            Some(&self.bindings),
        )
        .get(self.selected)
        .map(|shortcut| shortcut.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        action::Action,
        model::{
            data_query::DataQueryInput,
            relation::{RelationTab, RelationView},
            relation_edit::{RelationEditSession, RelationGridMode},
            tab::WorkspaceTab,
        },
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::HashSet;

    #[test]
    fn pane_help_uses_configured_binding_and_hides_disabled_commands() {
        let mut config = crate::config::AppConfig::default();
        config
            .keybindings
            .panes
            .insert("focus-pane-left".into(), vec!["Cmd+Ctrl+h".into()]);
        config
            .keybindings
            .panes
            .insert("focus-pane-right".into(), Vec::new());
        let bindings = config.keybindings.key_bindings().unwrap();
        let explorer = shortcut_catalog()
            .iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::FocusExplorer)
            .unwrap();
        let right = shortcut_catalog()
            .iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::FocusResultsFromL)
            .unwrap();

        assert_eq!(configured_sequence(explorer, Some(&bindings)), "Cmd+Ctrl+h");
        assert!(pane_binding_configured(explorer.id, Some(&bindings)));
        assert!(!pane_binding_configured(right.id, Some(&bindings)));
    }

    #[test]
    fn help_lists_selectable_quit_in_every_help_enabled_context() {
        let contexts = shortcut_catalog()
            .iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::Help)
            .unwrap()
            .contexts;
        for &context in contexts {
            let capabilities = ShortcutCapabilities::default();
            let rows = shortcuts(context, capabilities);
            let index = rows
                .iter()
                .position(|shortcut| shortcut.id == HelpShortcutId::Quit)
                .expect("quit row should be visible");
            assert_eq!(rows[index].sequence, "Ctrl-c");
            assert!(shortcut_is_executable(HelpShortcutId::Quit));

            let mut help = HelpState::new(context, capabilities);
            help.move_selection(index as isize, rows.len());
            assert_eq!(help.selected_id(), Some(HelpShortcutId::Quit));
            help.paste("ctrl-c");
            assert_eq!(help.selected_id(), Some(HelpShortcutId::Quit));
        }
    }

    #[test]
    fn help_search_applies_shared_cursor_and_deletion_edits() {
        let mut help = HelpState::new(ShortcutContext::Explorer, ShortcutCapabilities::default());
        help.paste("alpha beta");

        help.edit(TextInputEdit::MoveHome);
        help.edit(TextInputEdit::MoveRight);
        help.edit(TextInputEdit::Insert('-'));
        help.edit(TextInputEdit::MoveEnd);
        help.edit(TextInputEdit::DeletePreviousWord);

        assert_eq!(help.query.value(), "a-lpha ");
        assert_eq!(help.query.cursor(), 7);

        help.edit(TextInputEdit::MoveHome);
        help.edit(TextInputEdit::MoveRight);
        help.edit(TextInputEdit::Clear);
        assert_eq!(help.query.value(), "");
        assert_eq!(help.query.cursor(), 0);
    }

    #[test]
    fn catalog_is_single_complete_globally_unique_declaration() {
        let mut ids = HashSet::new();
        for shortcut in shortcut_catalog() {
            assert!(
                ids.insert(shortcut.id),
                "duplicate global ID {:?}",
                shortcut.id
            );
            assert!(!shortcut.contexts.is_empty());
            assert!(!shortcut.sequence.trim().is_empty());
            assert!(!shortcut.description.trim().is_empty());
        }
    }

    #[test]
    fn leader_menu_transaction_candidates_only_show_the_remaining_key() {
        let rows = prefix_shortcuts(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities {
                active_sql_console: true,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::EditorLeaderTransaction,
        );
        assert_eq!(rows.len(), 2);
        for (row, id, suffix) in [
            (&rows[0], HelpShortcutId::ToggleTransaction, "t"),
            (&rows[1], HelpShortcutId::TransactionControl, "c"),
        ] {
            assert_eq!(row.id, id);
            assert_eq!(row.suffix, Some(suffix));
            assert_eq!(
                format!("{}{suffix}", row.prefix.unwrap().display()),
                row.sequence
            );
        }
    }

    #[test]
    fn explicit_prefix_metadata_reconstructs_the_sequence() {
        for shortcut in shortcut_catalog() {
            match (shortcut.prefix, shortcut.suffix) {
                (Some(prefix), Some(suffix)) => {
                    assert_eq!(format!("{}{suffix}", prefix.display()), shortcut.sequence)
                }
                (None, None) => {}
                pair => panic!("incomplete prefix metadata for {:?}: {pair:?}", shortcut.id),
            }
        }
    }

    #[test]
    fn every_task_one_context_has_basic_rows() {
        for context in ALL_SHORTCUT_CONTEXTS {
            let capabilities = ShortcutCapabilities::relation_data();
            assert!(
                !shortcuts(*context, capabilities).is_empty(),
                "empty {context:?}"
            );
        }
    }

    #[test]
    fn relation_capability_changes_catalog_selection() {
        let unavailable = shortcuts(
            ShortcutContext::RelationDataBrowse,
            ShortcutCapabilities::default(),
        );
        let available = shortcuts(
            ShortcutContext::RelationDataBrowse,
            ShortcutCapabilities::relation_data(),
        );
        assert!(
            !unavailable
                .iter()
                .any(|row| row.id == HelpShortcutId::RelationWhere)
        );
        assert!(
            available
                .iter()
                .any(|row| row.id == HelpShortcutId::RelationWhere)
        );
        assert!(available.len() > unavailable.len());
    }

    #[test]
    fn explorer_and_dashboard_contexts_expose_dashboard_controls() {
        let explorer = shortcuts(ShortcutContext::Explorer, ShortcutCapabilities::default());
        assert!(
            explorer.iter().any(|row| {
                row.id == HelpShortcutId::OpenDashboard && row.sequence == "Space b"
            })
        );

        let dashboard = shortcuts(
            ShortcutContext::Dashboard,
            ShortcutCapabilities {
                focus: Focus::Results,
                relation_layout: true,
                ..ShortcutCapabilities::default()
            },
        );
        for id in [
            HelpShortcutId::DashboardToggleView,
            HelpShortcutId::DashboardRefresh,
            HelpShortcutId::DashboardTogglePolling,
            HelpShortcutId::FocusExplorer,
        ] {
            assert!(dashboard.iter().any(|row| row.id == id), "missing {id:?}");
        }
    }

    #[test]
    fn catalog_shortcuts_require_selected_capabilities() {
        let unavailable = shortcuts(ShortcutContext::Explorer, ShortcutCapabilities::default());
        assert!(!unavailable.iter().any(|row| matches!(
            row.id,
            HelpShortcutId::ExplorerCreateCatalog
                | HelpShortcutId::ExplorerEditCatalog
                | HelpShortcutId::ExplorerEditProfile
        )));

        let available = ShortcutCapabilities {
            profile_edit_available: true,
            catalog_create_available: true,
            catalog_edit_available: true,
            ..ShortcutCapabilities::default()
        };
        let rows = shortcuts(ShortcutContext::Explorer, available);
        assert!(
            rows.iter()
                .any(|row| row.id == HelpShortcutId::ExplorerCreateCatalog)
        );
        assert!(
            rows.iter()
                .any(|row| row.id == HelpShortcutId::ExplorerEditCatalog)
        );
        assert!(
            rows.iter()
                .any(|row| row.id == HelpShortcutId::ExplorerEditProfile)
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.id == HelpShortcutId::ExplorerCreateCatalog)
                .unwrap()
                .description,
            "add object"
        );
    }

    #[test]
    fn explorer_help_always_lists_group_shortcuts() {
        let rows = shortcuts(ShortcutContext::Explorer, ShortcutCapabilities::default());
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(
                    row.id,
                    HelpShortcutId::ExplorerCreateGroup
                        | HelpShortcutId::ExplorerEditGroup
                        | HelpShortcutId::ExplorerMoveToGroup
                        | HelpShortcutId::ExplorerDeleteGroup
                ))
                .count(),
            4
        );
    }

    #[test]
    fn goto_prefix_lists_move_to_group() {
        let rows = prefix_shortcuts(
            ShortcutContext::Explorer,
            ShortcutCapabilities::default(),
            ShortcutPrefix::Goto,
        );
        assert!(rows.iter().any(|row| {
            row.id == HelpShortcutId::ExplorerMoveToGroup
                && row.sequence == "gm"
                && row.suffix == Some("m")
        }));
    }

    #[test]
    fn sql_window_directions_match_three_pane_mapping() {
        let cases = [
            (Focus::Explorer, vec![HelpShortcutId::FocusEditorFromL]),
            (
                Focus::Editor,
                vec![HelpShortcutId::FocusExplorer, HelpShortcutId::FocusResults],
            ),
            (
                Focus::Results,
                vec![
                    HelpShortcutId::FocusExplorer,
                    HelpShortcutId::FocusEditorFromK,
                ],
            ),
        ];
        for (focus, expected) in cases {
            let capabilities = ShortcutCapabilities {
                focus,
                ..ShortcutCapabilities::default()
            };
            let actual = shortcuts(context_for_sql_focus(focus), capabilities)
                .into_iter()
                .filter(|row| {
                    matches!(
                        row.requirement,
                        ShortcutRequirement::PaneDirection(_)
                            | ShortcutRequirement::SqlPaneDirection(_)
                            | ShortcutRequirement::RelationPaneDirection(_)
                    )
                })
                .map(|row| row.id)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{focus:?}");
        }
    }

    #[test]
    fn relation_window_directions_match_two_pane_mapping() {
        let cases = [
            (Focus::Explorer, vec![HelpShortcutId::FocusResultsFromL]),
            (Focus::Results, vec![HelpShortcutId::FocusExplorer]),
        ];
        for (focus, expected) in cases {
            let capabilities = ShortcutCapabilities {
                focus,
                relation_layout: true,
                ..ShortcutCapabilities::default()
            };
            let context = match focus {
                Focus::Explorer => ShortcutContext::Explorer,
                Focus::Results => ShortcutContext::RelationDataBrowse,
                Focus::Editor => unreachable!(),
            };
            let actual = shortcuts(context, capabilities)
                .into_iter()
                .filter(|row| {
                    matches!(
                        row.requirement,
                        ShortcutRequirement::PaneDirection(_)
                            | ShortcutRequirement::SqlPaneDirection(_)
                            | ShortcutRequirement::RelationPaneDirection(_)
                    )
                })
                .map(|row| row.id)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{focus:?}");
        }
    }

    fn context_for_sql_focus(focus: Focus) -> ShortcutContext {
        match focus {
            Focus::Explorer => ShortcutContext::Explorer,
            Focus::Editor => ShortcutContext::EditorNormal,
            Focus::Results => ShortcutContext::SqlResultsData,
        }
    }

    #[test]
    fn contexts_share_one_id_entry_for_the_same_action() {
        let help = shortcut_catalog()
            .iter()
            .find(|row| row.id == HelpShortcutId::Help)
            .unwrap();
        assert!(help.contexts.contains(&ShortcutContext::Explorer));
        assert!(help.contexts.contains(&ShortcutContext::EditorNormal));
        assert!(help.contexts.contains(&ShortcutContext::SqlResultsData));
    }

    #[test]
    fn context_resolver_covers_modes_views_and_inputs() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Editor;
        assert_eq!(shortcut_context(&app), ShortcutContext::EditorInsert);
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(shortcut_context(&app), ShortcutContext::EditorNormal);
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char('V'),
            KeyModifiers::NONE,
        )));
        assert_eq!(shortcut_context(&app), ShortcutContext::EditorVisual);
        app.focus = Focus::Results;
        app.active_console_mut().result_view = ResultView::Output;
        assert_eq!(shortcut_context(&app), ShortcutContext::SqlOutput);
        app.active_console_mut().result_view = ResultView::Data;
        app.active_console_mut().query.focus = Some(DataQueryInput::Where);
        assert_eq!(shortcut_context(&app), ShortcutContext::DataQueryInput);
    }

    #[test]
    fn redis_contexts_separate_keys_find_and_value_views() {
        let mut app = App::new(Vec::new());
        app.tabs.push(WorkspaceTab::RedisBrowser(
            crate::model::redis_browser::RedisBrowserTab::new(
                uuid::Uuid::from_u128(1),
                crate::db::redis::types::RedisTarget {
                    profile_id: uuid::Uuid::from_u128(2),
                    database: 0,
                },
            ),
        ));
        app.active_tab = 1;
        app.focus = Focus::Results;

        assert_eq!(shortcut_context(&app), ShortcutContext::RedisKeys);
        if let Some(WorkspaceTab::RedisBrowser(tab)) = app.tabs.get_mut(1) {
            tab.focus = crate::model::redis_browser::RedisBrowserFocus::Preview;
        }
        assert_eq!(shortcut_context(&app), ShortcutContext::RedisPreview);
    }

    #[test]
    fn relation_contexts_are_distinct() {
        let mut app = App::new(Vec::new());
        let mut relation = RelationTab::new("items");
        relation.edit = Some(RelationEditSession::default());
        app.tabs = vec![WorkspaceTab::Relation(relation)];
        app.active_tab = 0;
        app.focus = Focus::Results;
        assert_eq!(shortcut_context(&app), ShortcutContext::RelationDataBrowse);
        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit.as_mut().unwrap().mode = RelationGridMode::VisualLine { anchor: 0 };
        assert_eq!(shortcut_context(&app), ShortcutContext::RelationDataVisual);
        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.view = RelationView::Ddl;
        assert_eq!(shortcut_context(&app), ShortcutContext::RelationDdl);
    }

    #[test]
    fn filtering_and_non_first_selection_use_stable_ids() {
        let mut help = HelpState::new(ShortcutContext::Explorer, ShortcutCapabilities::default());
        help.paste("move selection");
        let rows = filtered_shortcuts(help.context, help.capabilities, help.query.value());
        assert!(rows.len() > 1);
        help.move_selection(1, rows.len());
        assert_eq!(help.selected_id(), Some(HelpShortcutId::ExplorerMoveUp));
    }

    #[test]
    fn explorer_search_states_override_the_plain_explorer_context() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;
        app.explorer.open_find();
        assert_eq!(shortcut_context(&app), ShortcutContext::ExplorerFindEditing);
        app.explorer.find.as_mut().unwrap().phase = ExplorerSearchPhase::Confirmed;
        assert_eq!(
            shortcut_context(&app),
            ShortcutContext::ExplorerFindConfirmed
        );

        app.explorer.find = None;
        app.explorer.open_search(None, 1);
        assert_eq!(
            shortcut_context(&app),
            ShortcutContext::ExplorerCatalogSearchEditing
        );
        app.explorer.search.as_mut().unwrap().phase = ExplorerSearchPhase::Confirmed;
        assert_eq!(
            shortcut_context(&app),
            ShortcutContext::ExplorerCatalogSearchConfirmed
        );
    }

    #[test]
    fn representative_overlays_resolve_to_their_own_contexts() {
        let mut app = App::new(Vec::new());
        let cases = [
            (
                Overlay::Help(HelpState::new(
                    ShortcutContext::Explorer,
                    ShortcutCapabilities::default(),
                )),
                ShortcutContext::Help,
            ),
            (
                Overlay::RecordView(Default::default()),
                ShortcutContext::RecordView,
            ),
            (
                Overlay::ProfileAccess {
                    profile_id: uuid::Uuid::nil(),
                    selected: 0,
                    options: Vec::new(),
                },
                ShortcutContext::ProfileAccess,
            ),
            (
                Overlay::Message {
                    title: "notice".into(),
                    body: "body".into(),
                },
                ShortcutContext::Message,
            ),
            (
                Overlay::SubstituteConfirm { remaining: 1 },
                ShortcutContext::SubstituteConfirmation,
            ),
            (
                Overlay::TargetSelector {
                    candidates: Vec::new(),
                    selected: 0,
                    console_id: None,
                    execution: None,
                },
                ShortcutContext::TargetSelector,
            ),
            (
                Overlay::SqlEditorList(Default::default()),
                ShortcutContext::ConsoleManager,
            ),
            (
                Overlay::DeleteConsole {
                    console_id: uuid::Uuid::nil(),
                    focus: crate::model::workspace::DeleteConsoleFocus::Cancel,
                },
                ShortcutContext::DeleteConsoleConfirmation,
            ),
            (
                Overlay::PageSizeSelector {
                    relation: false,
                    selected: 0,
                },
                ShortcutContext::PageSizeSelector,
            ),
        ];
        for (overlay, expected) in cases {
            app.overlay = Some(overlay);
            assert_eq!(shortcut_context(&app), expected);
        }
    }

    #[test]
    fn console_manager_help_changes_with_mode() {
        let mut app = App::new(Vec::new());
        let id = app.active_console().id;
        let modes = [
            (SqlEditorListMode::Browse, ShortcutContext::ConsoleManager),
            (
                SqlEditorListMode::Search,
                ShortcutContext::ConsoleManagerSearch,
            ),
            (
                SqlEditorListMode::Rename {
                    console_id: id,
                    input: Default::default(),
                    error: None,
                },
                ShortcutContext::ConsoleManagerRename,
            ),
            (
                SqlEditorListMode::DeleteConfirm { console_id: id },
                ShortcutContext::ConsoleManagerDeleteConfirm,
            ),
        ];

        for (mode, context) in modes {
            app.overlay = Some(Overlay::SqlEditorList(
                crate::model::sql_editor_list::SqlEditorListState {
                    mode,
                    ..Default::default()
                },
            ));
            assert_eq!(shortcut_context(&app), context);
            assert!(!shortcuts(context, ShortcutCapabilities::default()).is_empty());
        }

        let browse = shortcuts(
            ShortcutContext::ConsoleManager,
            ShortcutCapabilities::default(),
        );
        assert!(browse.iter().any(|row| row.sequence == "a"));
        assert!(browse.iter().any(|row| row.sequence == "d"));
        assert!(browse.iter().any(|row| row.sequence == "r"));
        let search = shortcuts(
            ShortcutContext::ConsoleManagerSearch,
            ShortcutCapabilities::default(),
        );
        assert!(!search.iter().any(|row| row.sequence == "d"));
        assert!(!search.iter().any(|row| row.sequence == "r"));
        let rename = shortcuts(
            ShortcutContext::ConsoleManagerRename,
            ShortcutCapabilities::default(),
        );
        assert!(!rename.iter().any(|row| row.sequence == "d"));
        let delete = shortcuts(
            ShortcutContext::ConsoleManagerDeleteConfirm,
            ShortcutCapabilities::default(),
        );
        assert!(delete.iter().any(|row| row.sequence == "Enter/y"));
        assert!(!delete.iter().any(|row| row.sequence == "a"));
    }

    #[test]
    fn leader_help_lists_only_space_s_for_console_manager() {
        let rows = prefix_shortcuts(
            ShortcutContext::Explorer,
            ShortcutCapabilities::default(),
            ShortcutPrefix::Leader,
        );
        assert!(
            rows.iter().any(|row| {
                row.id == HelpShortcutId::OpenSqlEditors && row.sequence == "Space s"
            })
        );
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.sequence, "Space n" | "Space e"))
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.description.contains("first SQL console"))
        );

        let editor_rows = prefix_shortcuts(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities::default(),
            ShortcutPrefix::Leader,
        );
        assert!(
            editor_rows.iter().any(|row| {
                row.id == HelpShortcutId::OpenSqlEditors && row.sequence == "Space s"
            })
        );
    }

    #[test]
    fn task_eight_context_catalog_covers_completion_and_modal_controls() {
        let completion = shortcuts(
            ShortcutContext::DataQueryInput,
            ShortcutCapabilities::default(),
        );
        for sequence in ["Ctrl-n", "Ctrl-p", "Tab", "Enter", "Esc"] {
            assert!(
                completion
                    .iter()
                    .any(|row| { row.sequence == sequence || row.sequence.contains(sequence) }),
                "missing Data Query completion control {sequence}"
            );
        }

        for context in [
            ShortcutContext::ProfileAccess,
            ShortcutContext::Message,
            ShortcutContext::SubstituteConfirmation,
            ShortcutContext::ExecutionConfirmation,
            ShortcutContext::ManualCancelConfirmation,
            ShortcutContext::TransactionExitConfirmation,
            ShortcutContext::TransactionReviewConfirmation,
            ShortcutContext::ClearTransactionOutcomeConfirmation,
            ShortcutContext::TargetSelector,
            ShortcutContext::DeleteConsoleConfirmation,
        ] {
            assert!(
                !shortcuts(context, ShortcutCapabilities::default()).is_empty(),
                "empty modal context {context:?}"
            );
        }
    }

    #[test]
    fn task_eight_contexts_have_at_least_one_real_control_row() {
        for context in ALL_SHORTCUT_CONTEXTS {
            assert!(
                !shortcuts(*context, ShortcutCapabilities::relation_data()).is_empty(),
                "context lacks a catalog row: {context:?}"
            );
        }
    }

    #[test]
    fn task_eight_dynamic_contexts_prioritize_their_own_controls() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;
        app.explorer.open_find();
        assert_eq!(shortcut_context(&app), ShortcutContext::ExplorerFindEditing);
        assert!(
            footer_sequences(
                ShortcutContext::ExplorerFindEditing,
                ShortcutCapabilities::default()
            )
            .contains(&"Enter")
        );

        app.explorer.find = None;
        app.explorer.open_search(None, 1);
        app.explorer.search.as_mut().unwrap().phase = ExplorerSearchPhase::Confirmed;
        assert_eq!(
            shortcut_context(&app),
            ShortcutContext::ExplorerCatalogSearchConfirmed
        );

        app.focus = Focus::Results;
        app.active_console_mut().query.focus = Some(DataQueryInput::Where);
        app.active_console_mut().query.completion =
            Some(crate::model::data_query::DataQueryCompletion {
                candidates: Vec::new(),
                selected: 0,
                replace: crate::sql::TextRange::new(0, 0),
            });
        assert_eq!(shortcut_context(&app), ShortcutContext::DataQueryInput);
        let query = shortcuts(
            ShortcutContext::DataQueryInput,
            ShortcutCapabilities::default(),
        );
        assert!(query.iter().any(|row| row.sequence == "Ctrl-n"));

        app.overlay = Some(Overlay::RecordView(Default::default()));
        assert_eq!(shortcut_context(&app), ShortcutContext::RecordView);
        assert!(
            shortcuts(ShortcutContext::RecordView, ShortcutCapabilities::default())
                .iter()
                .any(|row| row.sequence == "gg")
        );

        app.overlay = Some(Overlay::Message {
            title: "notice".into(),
            body: "body".into(),
        });
        assert_eq!(shortcut_context(&app), ShortcutContext::Message);
        assert!(
            shortcuts(ShortcutContext::Message, ShortcutCapabilities::default())
                .iter()
                .any(|row| row.sequence == "Esc/q")
        );
    }

    #[test]
    fn relation_catalog_rows_are_limited_to_the_matching_state() {
        use crate::input::keymap::Keymap;

        let mut app = App::new(Vec::new());
        let mut relation = RelationTab::new("items");
        relation.edit = Some(RelationEditSession::default());
        app.tabs = vec![WorkspaceTab::Relation(relation)];
        app.active_tab = 0;
        app.focus = Focus::Results;

        let browse = shortcuts(
            ShortcutContext::RelationDataBrowse,
            shortcut_capabilities(&app),
        );
        let browse_ids = browse.iter().map(|row| row.id).collect::<Vec<_>>();
        assert!(browse_ids.contains(&HelpShortcutId::RelationDeleteRow));
        assert!(browse_ids.contains(&HelpShortcutId::RelationEditCell));
        assert!(!browse_ids.contains(&HelpShortcutId::RelationEditApply));
        assert!(!browse_ids.contains(&HelpShortcutId::RelationVisualMove));
        assert!(!browse_ids.contains(&HelpShortcutId::RelationBusyData));

        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit.as_mut().unwrap().mode =
            RelationGridMode::EditCell(Box::new(crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            }));
        let edit = shortcuts(
            ShortcutContext::RelationDataEdit,
            shortcut_capabilities(&app),
        );
        let edit_ids = edit.iter().map(|row| row.id).collect::<Vec<_>>();
        assert!(edit_ids.contains(&HelpShortcutId::RelationEditApply));
        assert!(edit_ids.contains(&HelpShortcutId::RelationEditCancel));
        assert!(!edit_ids.contains(&HelpShortcutId::RelationEditCell));
        assert_eq!(
            Keymap::default().map(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                &app,
            ),
            Some(crate::action::Action::RelationEditConfirm)
        );

        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit.as_mut().unwrap().mode = RelationGridMode::VisualLine { anchor: 0 };
        let visual = shortcuts(
            ShortcutContext::RelationDataVisual,
            shortcut_capabilities(&app),
        );
        let visual_ids = visual.iter().map(|row| row.id).collect::<Vec<_>>();
        assert!(visual_ids.contains(&HelpShortcutId::RelationVisualMove));
        assert!(visual_ids.contains(&HelpShortcutId::RelationVisualYank));
        assert!(visual_ids.contains(&HelpShortcutId::RelationVisualDelete));
        assert!(!visual_ids.contains(&HelpShortcutId::RelationEditCell));

        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit.as_mut().unwrap().mode = RelationGridMode::Busy;
        let busy = shortcuts(
            ShortcutContext::RelationDataBusy,
            shortcut_capabilities(&app),
        );
        let busy_ids = busy.iter().map(|row| row.id).collect::<Vec<_>>();
        assert!(busy_ids.contains(&HelpShortcutId::RelationBusyData));
        assert!(busy_ids.contains(&HelpShortcutId::RelationBusyRefresh));
        assert!(!busy_ids.contains(&HelpShortcutId::RelationEditCell));
        assert!(!busy_ids.contains(&HelpShortcutId::RelationDeleteRow));
    }

    #[test]
    fn relation_state_catalog_rows_match_keymap_actions() {
        use crate::input::keymap::Keymap;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        fn relation_app(mode: RelationGridMode) -> App {
            let mut app = App::new(Vec::new());
            let mut relation = RelationTab::new("items");
            let mut edit = RelationEditSession::default();
            edit.mode = mode;
            relation.edit = Some(edit);
            app.tabs = vec![WorkspaceTab::Relation(relation)];
            app.active_tab = 0;
            app.focus = Focus::Results;
            app
        }

        let browse = relation_app(RelationGridMode::Browse);
        let browse_rows = shortcuts(
            ShortcutContext::RelationDataBrowse,
            shortcut_capabilities(&browse),
        );
        for (sequence, id, expected) in [
            (
                "e",
                HelpShortcutId::RelationEditCell,
                Action::RelationEditCell,
            ),
            (
                "a",
                HelpShortcutId::RelationInsertRow,
                Action::RelationInsertRow,
            ),
            (
                "V",
                HelpShortcutId::RelationVisualLine,
                Action::RelationVisualLine,
            ),
            ("p", HelpShortcutId::RelationPaste, Action::RelationPaste),
        ] {
            let row = browse_rows
                .iter()
                .find(|row| row.id == id)
                .expect("browse row");
            assert!(row.footer_priority.is_some());
            let code = sequence.chars().next().unwrap();
            assert_eq!(
                Keymap::default().map(
                    KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE),
                    &browse
                ),
                Some(expected)
            );
        }

        let edit = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let edit_rows = shortcuts(
            ShortcutContext::RelationDataEdit,
            shortcut_capabilities(&edit),
        );
        assert!(
            edit_rows
                .iter()
                .any(|row| row.id == HelpShortcutId::RelationEditApply)
        );
        assert!(
            edit_rows
                .iter()
                .any(|row| row.id == HelpShortcutId::RelationEditCancel)
        );
        assert!(
            !edit_rows
                .iter()
                .any(|row| row.id == HelpShortcutId::RelationEditCell)
        );
        assert_eq!(
            Keymap::default().map(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                &edit
            ),
            Some(Action::RelationEditDeletePreviousWord)
        );
    }

    fn footer_sequences(
        context: ShortcutContext,
        capabilities: ShortcutCapabilities,
    ) -> Vec<&'static str> {
        footer_shortcuts(context, capabilities)
            .into_iter()
            .map(|shortcut| shortcut.sequence)
            .collect()
    }

    #[test]
    fn footer_primary_contexts_have_exact_priority_order() {
        let standard = ShortcutCapabilities {
            data_query_available: true,
            record_view_available: true,
            ..ShortcutCapabilities::default()
        };
        let cases = [
            (
                ShortcutContext::Explorer,
                vec!["j", "k", "h", "l", "Enter", "/", "f", "r", "? (also F1)"],
            ),
            (
                ShortcutContext::EditorNormal,
                vec!["i", "R / F5", "Space f", "Space y", "? (also F1)"],
            ),
            (
                ShortcutContext::EditorInsert,
                vec!["Esc", "Ctrl-Space", "Ctrl-w", "F5", "? (also F1)"],
            ),
            (
                ShortcutContext::EditorVisual,
                vec!["y", "Esc", "R / F5", "Space f", "? (also F1)"],
            ),
            (
                ShortcutContext::SqlResultsData,
                vec!["h", "j", "k", "l", "v", "y", "Y", "o", "/", "? (also F1)"],
            ),
            (
                ShortcutContext::SqlOutput,
                vec!["j/k", "gg/G", "/", "v/V", "y", "o", "? (also F1)"],
            ),
            (
                ShortcutContext::RelationDdl,
                vec!["j/k", "gg/G", "/", "v/V", "y", "p", "r", "? (also F1)"],
            ),
            (
                ShortcutContext::RecordView,
                vec!["j/k", "h/l", "gg/G", "Esc/q/v"],
            ),
            (
                ShortcutContext::DataQueryInput,
                vec!["type / Backspace", "Enter", "Esc", "Tab/Shift-Tab"],
            ),
        ];
        for (context, expected) in cases {
            assert_eq!(footer_sequences(context, standard), expected, "{context:?}");
        }
    }

    #[test]
    fn footer_relation_modes_and_availability_are_exact() {
        let editable = ShortcutCapabilities {
            relation_data: true,
            relation_edit_available: true,
            data_query_available: true,
            record_view_available: true,
            ..ShortcutCapabilities::default()
        };
        assert_eq!(
            footer_sequences(ShortcutContext::RelationDataBrowse, editable),
            vec!["h", "j", "k", "l", "y", "e", "a", "V", "p", "Ctrl-s"]
        );
        assert_eq!(
            footer_sequences(ShortcutContext::RelationDataEdit, editable),
            vec![
                "Enter",
                "Esc",
                "type / Backspace",
                "Left/Right Space t/f",
                "Left/Right  [/]  type",
                "Alt-N",
                "Alt-D",
                "Alt-V",
                "Ctrl-w",
            ]
        );
        assert_eq!(
            footer_sequences(ShortcutContext::RelationDataVisual, editable),
            vec!["j/k", "y", "d", "V"]
        );

        let read_only = ShortcutCapabilities {
            relation_data: true,
            data_query_available: true,
            record_view_available: true,
            ..ShortcutCapabilities::default()
        };
        let rows = footer_sequences(ShortcutContext::RelationDataBrowse, read_only);
        assert_eq!(rows, vec!["h", "j", "k", "l", "v", "y", "Y", "/", "s", "r"]);
        for editing in ["e", "a", "V", "p", "Ctrl-s"] {
            assert!(!rows.contains(&editing));
        }
    }

    #[test]
    fn relation_data_help_does_not_list_console_transaction_control() {
        let capabilities = ShortcutCapabilities {
            relation_data: true,
            relation_edit_available: true,
            ..ShortcutCapabilities::default()
        };
        let rows = shortcuts(ShortcutContext::RelationDataBrowse, capabilities);

        assert!(
            !rows
                .iter()
                .any(|shortcut| shortcut.id == HelpShortcutId::TransactionControl)
        );
        assert!(
            rows.iter()
                .any(|shortcut| shortcut.id == HelpShortcutId::RelationCommit)
        );
        assert!(shortcut_is_executable(HelpShortcutId::RelationCommit));
    }

    #[test]
    fn column_details_footer_lists_navigation_toggle_and_exit() {
        assert_eq!(
            footer_sequences(
                ShortcutContext::CatalogEditorColumnDetails,
                ShortcutCapabilities::default()
            ),
            vec![
                "Tab/Shift-Tab/Up/Down",
                "type / Backspace",
                "Space",
                "Enter",
                "Esc"
            ]
        );
    }

    #[test]
    fn footer_dynamic_capabilities_hide_unavailable_actions() {
        let unavailable = ShortcutCapabilities::default();
        let sql = footer_sequences(ShortcutContext::SqlResultsData, unavailable);
        assert!(!sql.contains(&"v"));
        assert!(!sql.contains(&"/"));

        let relation = ShortcutCapabilities {
            relation_data: true,
            ..ShortcutCapabilities::default()
        };
        let rows = footer_sequences(ShortcutContext::RelationDataBrowse, relation);
        assert!(!rows.contains(&"e"));
        assert!(!rows.contains(&"v"));
        assert!(!rows.contains(&"/"));
    }

    #[test]
    fn app_capabilities_reflect_query_edit_record_and_busy_state() {
        use crate::db::value::CellValue;

        let mut app = App::new(Vec::new());
        app.tabs = vec![WorkspaceTab::Relation(RelationTab::new("items"))];
        app.active_tab = 0;
        app.focus = Focus::Results;

        let read_only = shortcut_capabilities(&app);
        assert!(read_only.data_query_available);
        assert!(!read_only.relation_edit_available);
        assert!(!read_only.record_view_available);

        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit = Some(RelationEditSession::from_rows(vec![vec![
            CellValue::Integer(1),
        ]]));
        let editable = shortcut_capabilities(&app);
        assert!(editable.relation_edit_available);
        assert!(editable.record_view_available);

        let WorkspaceTab::Relation(tab) = &mut app.tabs[0] else {
            unreachable!()
        };
        tab.edit.as_mut().unwrap().mode = RelationGridMode::Busy;
        let busy = shortcut_capabilities(&app);
        assert!(!busy.relation_edit_available);
        let rows = footer_sequences(ShortcutContext::RelationDataBrowse, busy);
        assert!(!rows.contains(&"i"));
        assert!(!rows.contains(&"Ctrl-s"));
    }

    #[test]
    fn footer_modal_contexts_have_actual_controls() {
        let capabilities = ShortcutCapabilities::default();
        let cases = [
            (
                ShortcutContext::ExplorerFindEditing,
                vec!["type / Backspace / Ctrl-w / Ctrl-u", "Enter"],
            ),
            (ShortcutContext::ExplorerFindConfirmed, vec!["n", "N"]),
            (
                ShortcutContext::ExplorerCatalogSearchEditing,
                vec!["type / Backspace / Ctrl-w / Ctrl-u", "Enter"],
            ),
            (
                ShortcutContext::ExplorerCatalogSearchConfirmed,
                vec!["n", "N"],
            ),
            (
                ShortcutContext::ProfileManagerForm,
                vec!["Tab/Shift-Tab", "Space", "Enter", "Esc"],
            ),
            (
                ShortcutContext::ProfileManagerScope,
                vec!["j/k", "Space", "r", "Esc/Enter"],
            ),
            (ShortcutContext::ProfileManagerDelete, vec!["Enter", "Esc"]),
            (
                ShortcutContext::ConsoleManager,
                vec!["j/k or Up/Down", "Enter", "d", "r", "/", "a", "Esc"],
            ),
            (
                ShortcutContext::ConsoleManagerSearch,
                vec![
                    "Up/Down",
                    "type / Backspace / Delete / Ctrl-w / Ctrl-u",
                    "Enter",
                    "Esc",
                ],
            ),
            (
                ShortcutContext::ConsoleManagerRename,
                vec![
                    "type / Backspace / Delete / Ctrl-w / Ctrl-u",
                    "Enter",
                    "Esc",
                ],
            ),
            (
                ShortcutContext::ConsoleManagerDeleteConfirm,
                vec!["Enter/y", "Esc/n/q"],
            ),
            (
                ShortcutContext::Help,
                vec!["type / Backspace", "Up/Down", "Enter", "Esc"],
            ),
            (
                ShortcutContext::ProfileAccess,
                vec!["j/k", "Enter", "Esc/q"],
            ),
            (ShortcutContext::Message, vec!["Esc/q"]),
            (
                ShortcutContext::SubstituteConfirmation,
                vec!["y/n/a/l", "Esc/q"],
            ),
            (
                ShortcutContext::ExecutionConfirmation,
                vec!["Enter", "Esc/n/q", "Tab/Shift-Tab/Left/Right"],
            ),
            (
                ShortcutContext::ManualCancelConfirmation,
                vec!["Enter/c", "Esc/k", "Tab/Left/Right"],
            ),
            (
                ShortcutContext::TransactionExitConfirmation,
                vec!["a/r/c/Enter", "Esc/n", "Tab/Left/Right"],
            ),
            (
                ShortcutContext::TransactionReviewConfirmation,
                vec![
                    "hjkl / gg / G / PageUp/PageDown",
                    "v/V/y or mouse drag",
                    "Tab/Shift-Tab",
                ],
            ),
            (
                ShortcutContext::ClearTransactionOutcomeConfirmation,
                vec!["Enter", "Esc", "Tab/Left/Right"],
            ),
            (ShortcutContext::TargetSelector, vec!["j/k", "Enter", "Esc"]),
            (
                ShortcutContext::DeleteConsoleConfirmation,
                vec!["Tab, then Enter", "Esc"],
            ),
        ];
        for (context, expected) in cases {
            assert_eq!(
                footer_sequences(context, capabilities),
                expected,
                "{context:?}"
            );
        }
    }

    #[test]
    fn catalog_editor_help_matches_field_navigation_and_toggle_scope() {
        let capabilities = ShortcutCapabilities::default();
        assert_eq!(
            footer_sequences(ShortcutContext::CatalogEditorForm, capabilities),
            vec!["Tab/Shift-Tab/Up/Down", "type / Backspace", "Enter", "Esc"]
        );
        assert_eq!(
            footer_sequences(ShortcutContext::CatalogEditorColumnDetails, capabilities),
            vec![
                "Tab/Shift-Tab/Up/Down",
                "type / Backspace",
                "Space",
                "Enter",
                "Esc"
            ]
        );
        let toggle = shortcuts(ShortcutContext::CatalogEditorColumnDetails, capabilities)
            .into_iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::CatalogEditorColumnDetailsToggle)
            .expect("column details toggle help row");
        assert_eq!(toggle.description, "toggle Nullable/Identity");
    }

    #[test]
    fn insert_footer_ctrl_w_is_an_editor_command_not_a_window_prefix() {
        let rows = footer_shortcuts(
            ShortcutContext::EditorInsert,
            ShortcutCapabilities::default(),
        );
        let ctrl_w = rows.iter().find(|row| row.sequence == "Ctrl-w").unwrap();
        assert_eq!(ctrl_w.description, "delete previous word");
        assert_eq!(ctrl_w.prefix, None);
    }

    #[test]
    fn sql_output_footer_never_describes_cells() {
        for row in footer_shortcuts(ShortcutContext::SqlOutput, ShortcutCapabilities::default()) {
            assert!(!row.description.to_lowercase().contains("cell"));
        }
    }

    #[test]
    fn relation_browse_lists_cell_copy_without_row_yank() {
        let rows = shortcuts(
            ShortcutContext::RelationDataBrowse,
            ShortcutCapabilities::relation_data(),
        );
        assert!(
            !rows
                .iter()
                .any(|row| row.id == HelpShortcutId::ResultsCopyCell)
        );
        let copy = rows
            .iter()
            .find(|row| row.id == HelpShortcutId::RelationCopyCell)
            .expect("relation copy cell");
        assert_eq!(copy.sequence, "y");
        assert_eq!(copy.description, "copy selected cell");

        let footer = footer_sequences(
            ShortcutContext::RelationDataBrowse,
            ShortcutCapabilities {
                relation_data: true,
                data_query_available: true,
                record_view_available: true,
                ..ShortcutCapabilities::default()
            },
        );
        assert!(footer.contains(&"y"));
    }

    #[test]
    fn relation_catalog_executable_rows_match_physical_keymap() {
        use crate::input::keymap::Keymap;

        let mut app = App::new(Vec::new());
        app.tabs = vec![WorkspaceTab::Relation(RelationTab::new("items"))];
        app.active_tab = 0;
        app.focus = Focus::Results;
        let capabilities = shortcut_capabilities(&app);
        let rows = shortcuts(ShortcutContext::RelationDataBrowse, capabilities);

        for (code, id) in [
            (KeyCode::Enter, HelpShortcutId::RelationApplyInputs),
            (KeyCode::Char('['), HelpShortcutId::RelationResizeLeft),
            (KeyCode::Char(']'), HelpShortcutId::RelationResizeRight),
        ] {
            let row = rows.iter().find(|row| row.id == id).expect("catalog row");
            assert!(!row.executable, "unexpected executable {id:?}");
            assert_eq!(
                Keymap::default().map(KeyEvent::new(code, KeyModifiers::NONE), &app),
                None
            );
        }

        let reset = rows
            .iter()
            .find(|row| row.id == HelpShortcutId::RelationResetWidth)
            .expect("reset width row");
        assert!(reset.executable);
        assert_eq!(
            Keymap::default().map(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE), &app),
            Some(Action::GridResetColumnWidth)
        );
    }

    #[test]
    fn relation_busy_footer_rows_each_match_keymap_and_exclude_browse_actions() {
        use crate::input::keymap::Keymap;

        let mut app = App::new(Vec::new());
        let mut relation = RelationTab::new("items");
        let mut edit = RelationEditSession::default();
        edit.mode = RelationGridMode::Busy;
        relation.edit = Some(edit);
        app.tabs = vec![WorkspaceTab::Relation(relation)];
        app.active_tab = 0;
        app.focus = Focus::Results;

        assert_eq!(shortcut_context(&app), ShortcutContext::RelationDataBusy);
        let capabilities = shortcut_capabilities(&app);
        assert_eq!(
            footer_sequences(ShortcutContext::RelationDataBusy, capabilities),
            vec!["p", "r", "? (also F1)"]
        );
        assert!(
            !footer_sequences(ShortcutContext::RelationDataBusy, capabilities).contains(&"Ctrl-c")
        );
        assert_eq!(
            Keymap::default().map(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), &app),
            Some(Action::ShowHelp)
        );
        for code in [
            KeyCode::Char('e'),
            KeyCode::Char('a'),
            KeyCode::Char('V'),
            KeyCode::Char('y'),
            KeyCode::Char('d'),
        ] {
            assert_eq!(
                Keymap::default().map(KeyEvent::new(code, KeyModifiers::NONE), &app),
                None
            );
        }
        assert_eq!(
            Keymap::default().map(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE), &app),
            Some(Action::SetRelationView(
                crate::model::relation::RelationView::Data
            ))
        );
        assert_eq!(
            Keymap::default().map(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), &app),
            Some(Action::RefreshActiveRelation)
        );
    }

    fn prefix_ids(
        context: ShortcutContext,
        capabilities: ShortcutCapabilities,
        prefix: ShortcutPrefix,
    ) -> Vec<HelpShortcutId> {
        prefix_shortcuts(context, capabilities, prefix)
            .into_iter()
            .map(|shortcut| shortcut.id)
            .collect()
    }

    #[test]
    fn prefix_candidates_match_leader_and_window_availability() {
        let leader = prefix_ids(
            ShortcutContext::SqlResultsData,
            ShortcutCapabilities {
                active_sql_console: true,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Leader,
        );
        assert_eq!(
            leader,
            vec![
                HelpShortcutId::OpenDashboard,
                HelpShortcutId::RunSql,
                HelpShortcutId::RunAllSql,
                HelpShortcutId::CloseTab,
                HelpShortcutId::FocusExplorerLeader,
                HelpShortcutId::DeleteConsole,
                HelpShortcutId::OpenSqlEditors,
                HelpShortcutId::ResultsCopyRowWithHeaders,
                HelpShortcutId::OpenNotificationHistoryLeader,
            ]
        );
        assert!(!leader.contains(&HelpShortcutId::ToggleTransaction));
        assert!(!leader.contains(&HelpShortcutId::TransactionControl));
        assert!(leader.contains(&HelpShortcutId::FocusExplorerLeader));
        let editor_leader_prefix = prefix_ids(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities {
                active_sql_console: true,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Leader,
        );
        assert!(editor_leader_prefix.contains(&HelpShortcutId::OpenSqlEditors));
        let editor_leader = prefix_ids(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities::default(),
            ShortcutPrefix::EditorLeader,
        );
        assert_eq!(
            editor_leader,
            vec![
                HelpShortcutId::EditorFormat,
                HelpShortcutId::EditorCopyStatement,
                HelpShortcutId::EditorCopyBuffer,
                HelpShortcutId::OpenTargetSelector,
                HelpShortcutId::ToggleTransaction,
                HelpShortcutId::TransactionControl,
                HelpShortcutId::OpenNotificationHistoryLeader,
                HelpShortcutId::OpenDatabaseSelector,
            ]
        );

        assert!(
            shortcuts(
                ShortcutContext::EditorNormal,
                ShortcutCapabilities::default()
            )
            .iter()
            .any(|shortcut| shortcut.id == HelpShortcutId::OpenNotificationHistoryLeader)
        );
        assert!(
            !shortcuts(
                ShortcutContext::EditorInsert,
                ShortcutCapabilities::default()
            )
            .iter()
            .any(|shortcut| shortcut.id == HelpShortcutId::OpenNotificationHistoryLeader)
        );

        let mut config = crate::config::AppConfig::default();
        config
            .keybindings
            .global
            .insert("notification-history".into(), vec!["F10".into()]);
        let bindings = config.keybindings.key_bindings().unwrap();
        let notification = shortcut_catalog()
            .iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::OpenNotificationHistory)
            .expect("notification history shortcut");
        assert_eq!(configured_sequence(notification, Some(&bindings)), "F10");

        let sql_editor = prefix_ids(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities {
                focus: Focus::Editor,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Window,
        );
        assert_eq!(
            sql_editor,
            vec![
                HelpShortcutId::FocusExplorer,
                HelpShortcutId::FocusResults,
                HelpShortcutId::CyclePaneFocus,
                HelpShortcutId::TogglePaneMaximized,
                HelpShortcutId::ResizeHeightIncrease,
                HelpShortcutId::ResizeHeightDecrease,
                HelpShortcutId::ResizeWidthIncrease,
                HelpShortcutId::ResizeWidthDecrease,
                HelpShortcutId::ResetPaneSizes,
            ]
        );
        let sql_results = prefix_ids(
            ShortcutContext::SqlResultsData,
            ShortcutCapabilities {
                focus: Focus::Results,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Window,
        );
        assert_eq!(
            sql_results,
            vec![
                HelpShortcutId::FocusExplorer,
                HelpShortcutId::FocusEditorFromK,
                HelpShortcutId::CyclePaneFocus,
                HelpShortcutId::TogglePaneMaximized,
                HelpShortcutId::ResizeHeightIncrease,
                HelpShortcutId::ResizeHeightDecrease,
                HelpShortcutId::ResizeWidthIncrease,
                HelpShortcutId::ResizeWidthDecrease,
                HelpShortcutId::ResetPaneSizes,
            ]
        );
    }

    #[test]
    fn leader_run_candidates_require_an_active_sql_console() {
        let empty = prefix_shortcuts(
            ShortcutContext::EditorNormal,
            ShortcutCapabilities {
                active_sql_console: false,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Leader,
        );
        assert!(
            !empty.iter().any(|row| {
                matches!(row.id, HelpShortcutId::RunSql | HelpShortcutId::RunAllSql)
            })
        );

        let sql = prefix_shortcuts(
            ShortcutContext::SqlResultsData,
            ShortcutCapabilities {
                active_sql_console: true,
                ..ShortcutCapabilities::default()
            },
            ShortcutPrefix::Leader,
        );
        assert!(sql.iter().any(|row| row.id == HelpShortcutId::RunSql));
        assert!(sql.iter().any(|row| row.id == HelpShortcutId::RunAllSql));

        for context in [
            ShortcutContext::RelationDataBrowse,
            ShortcutContext::RelationDdl,
        ] {
            assert!(
                prefix_shortcuts(
                    context,
                    ShortcutCapabilities {
                        active_sql_console: true,
                        ..ShortcutCapabilities::default()
                    },
                    ShortcutPrefix::Leader,
                )
                .iter()
                .all(|row| !matches!(row.id, HelpShortcutId::RunSql | HelpShortcutId::RunAllSql))
            );
        }
    }

    #[test]
    fn prefix_candidates_match_goto_align_tabs_and_relation_operations() {
        assert_eq!(
            prefix_ids(
                ShortcutContext::Explorer,
                ShortcutCapabilities::default(),
                ShortcutPrefix::Goto,
            ),
            vec![
                HelpShortcutId::ExplorerFirst,
                HelpShortcutId::ExplorerMoveToGroup,
            ]
        );
        assert_eq!(
            prefix_ids(
                ShortcutContext::SqlResultsData,
                ShortcutCapabilities::default(),
                ShortcutPrefix::Goto,
            ),
            vec![
                HelpShortcutId::ResultsFirstRow,
                HelpShortcutId::NextTab,
                HelpShortcutId::PreviousTab,
            ]
        );
        assert_eq!(
            prefix_ids(
                ShortcutContext::Explorer,
                ShortcutCapabilities::default(),
                ShortcutPrefix::ExplorerAlign,
            ),
            vec![
                HelpShortcutId::ExplorerAlignMiddle,
                HelpShortcutId::ExplorerAlignTop,
                HelpShortcutId::ExplorerAlignBottom,
            ]
        );
        assert_eq!(
            prefix_ids(
                ShortcutContext::SqlResultsData,
                ShortcutCapabilities::default(),
                ShortcutPrefix::GridAlign,
            ),
            vec![
                HelpShortcutId::ResultsAlignMiddle,
                HelpShortcutId::ResultsAlignTop,
                HelpShortcutId::ResultsAlignBottom,
            ]
        );
        assert_eq!(
            prefix_ids(
                ShortcutContext::RelationDataBrowse,
                ShortcutCapabilities::relation_data(),
                ShortcutPrefix::RelationDelete,
            ),
            vec![HelpShortcutId::RelationDeleteRow]
        );
    }

    #[test]
    fn prefix_candidates_match_record_view_and_reject_unknown_context_pairs() {
        assert_eq!(
            prefix_ids(
                ShortcutContext::RecordView,
                ShortcutCapabilities::default(),
                ShortcutPrefix::RecordViewGoto,
            ),
            vec![HelpShortcutId::RecordFirstField]
        );
        assert!(
            prefix_shortcuts(
                ShortcutContext::SqlOutput,
                ShortcutCapabilities::default(),
                ShortcutPrefix::RelationDelete,
            )
            .is_empty()
        );
    }

    #[test]
    fn profile_scope_loading_footer_matches_keymap_availability() {
        use crate::input::keymap::Keymap;
        use crate::model::profile_manager::{DiscoveryFingerprint, ProfileManagerState};

        let profile = crate::profile::import_connection_url(":memory:", Some("scope"))
            .unwrap()
            .profile;
        let fingerprint = DiscoveryFingerprint::for_profile(&profile, false, 0);
        let mut app = App::new(vec![profile]);
        let mut manager = ProfileManagerState::new(false);
        manager.page = ProfileManagerPage::Scope;
        manager.begin_scope_discovery(1, fingerprint);
        app.profile_manager = Some(manager);
        app.overlay = Some(Overlay::ProfileManager);

        let capabilities = shortcut_capabilities(&app);
        assert_eq!(
            footer_sequences(ShortcutContext::ProfileManagerScope, capabilities),
            vec!["j/k", "Esc/Enter"]
        );
        let mut keymap = Keymap::default();
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), &app),
            None
        );
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), &app),
            None
        );
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &app),
            Some(Action::ProfileScopeMove(1))
        );
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &app),
            Some(Action::ProfileScopeBack)
        );
    }
}
