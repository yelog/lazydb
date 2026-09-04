use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use lazydb::model::text_input::TextInputEdit;
use lazydb::{
    action::Action,
    app::App,
    db::ServerInfo,
    input::keymap::{Keymap, map_paste},
    model::{
        data_query::{DataQueryCandidate, DataQueryCompletion, DataQueryInput},
        editor::EditorMode,
        explorer::ExplorerNodeId,
        profile_manager::ProfileField,
        relation::RelationTab,
        tab::{CompletionPopup, WorkspaceTab},
        workspace::{Focus, Overlay, QueryStatus},
    },
    profile::{ConnectionProfile, import_connection_url},
};
use uuid::Uuid;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn normal_mode_ctrl_r_routes_to_editor_redo() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    assert_eq!(app.active_editor_mode(), EditorMode::Normal);

    let mut keymap = Keymap::default();
    let redo = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);

    assert_eq!(keymap.map(redo, &app), Some(Action::EditorKey(redo)));
}

#[test]
fn console_manager_browse_and_search_keys_are_mode_aware() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenSqlEditorList);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::SqlEditorListMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::SqlEditorListCreate)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('x')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('/')), &app),
        Some(Action::SqlEditorListSearchStart)
    );
    app.update(Action::SqlEditorListSearchStart);

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::SqlEditorListInputInsert('j'))
    );
    assert_eq!(
        keymap.map(ctrl('w'), &app),
        Some(Action::SqlEditorListInputDeletePreviousWord)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Delete), &app),
        Some(Action::SqlEditorListInputDelete)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SqlEditorListActivate)
    );
}

#[test]
fn console_manager_delete_keys_are_confirmation_aware() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenSqlEditorList);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::SqlEditorListDeleteRequest)
    );
    app.update(Action::SqlEditorListDeleteRequest);
    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::SqlEditorListDeleteConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::SqlEditorListDeleteCancel)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::SqlEditorListDeleteCancel)
    );
}

#[test]
fn console_manager_rename_keys_use_shared_text_input_actions() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenSqlEditorList);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::SqlEditorListRenameStart)
    );
    app.update(Action::SqlEditorListRenameStart);

    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::SqlEditorListInputInsert('a'))
    );
    assert_eq!(
        keymap.map(ctrl('w'), &app),
        Some(Action::SqlEditorListInputDeletePreviousWord)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Home), &app),
        Some(Action::SqlEditorListInputMoveHome)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SqlEditorListRenameCommit)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::SqlEditorListCancel)
    );
}

#[test]
fn explorer_s_opens_profile_access_menu() {
    let profile = profile("access");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenProfileAccess)
    );
    app.update(Action::OpenProfileAccess);
    assert!(matches!(
        app.overlay,
        Some(Overlay::ProfileAccess { profile_id: id, .. }) if id == profile_id
    ));
}

#[test]
fn profile_access_menu_navigates_and_cancels() {
    let profile = profile("access");
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.update(Action::OpenProfileAccess);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::ProfileAccessMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::ProfileAccessCancel)
    );
}

fn ctrl(character: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
}

fn table_editor_app() -> App {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Group {
            schema: lazydb::db::catalog::CatalogId::new(
                Uuid::nil(),
                lazydb::db::catalog::CatalogKind::Schema,
                ["app", "public"],
            ),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Table,
            ),
            label: "Table".into(),
        }],
    );
    assert!(
        editor.select_object_type(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Table,
        ))
    );
    app.catalog_editor = Some(editor);
    app.overlay = Some(Overlay::CatalogEditor);
    app
}

#[test]
fn table_editor_keymap_dispatches_by_focus_region() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();

    let set_focus = |app: &mut App, focus| {
        let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        else {
            panic!("table draft expected");
        };
        draft.focus = focus;
    };

    set_focus(
        &mut app,
        lazydb::model::catalog_editor::TableEditorFocus::Columns,
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::CatalogEditorAddTableColumn)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::CatalogEditorOpenTableColumnDetails)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancel)
    );

    app.update(Action::CatalogEditorOpenTableColumnDetails);
    set_focus(
        &mut app,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Comment,
        ),
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancelTableColumnDetails)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::CatalogEditorInsert('a'))
    );

    app.update(Action::CatalogEditorCancelTableColumnDetails);
    set_focus(
        &mut app,
        lazydb::model::catalog_editor::TableEditorFocus::General(
            lazydb::model::catalog_editor::TableGeneralField::Name,
        ),
    );
    assert_eq!(
        keymap.map(ctrl('a'), &app),
        Some(Action::CatalogEditorMoveHome)
    );
    assert_eq!(
        keymap.map(ctrl('e'), &app),
        Some(Action::CatalogEditorMoveEnd)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
}

#[test]
fn catalog_table_form_accepts_paste_as_one_text_edit() {
    let app = table_editor_app();
    assert_eq!(
        map_paste("orders".into(), &app),
        vec![Action::CatalogEditorPaste("orders".into())]
    );
}

#[test]
fn table_editor_action_buttons_keep_enter_and_space_semantics() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    for action_field in [
        lazydb::model::catalog_editor::TableActionField::AddColumn,
        lazydb::model::catalog_editor::TableActionField::RemoveColumn,
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::Cancel,
    ] {
        let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        else {
            panic!("table draft expected");
        };
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Action(action_field);
        let expected = match action_field {
            lazydb::model::catalog_editor::TableActionField::AddColumn => {
                Action::CatalogEditorAddTableColumn
            }
            lazydb::model::catalog_editor::TableActionField::RemoveColumn => {
                Action::CatalogEditorRemoveTableColumn
            }
            lazydb::model::catalog_editor::TableActionField::Review => Action::CatalogEditorPreview,
            lazydb::model::catalog_editor::TableActionField::Cancel => Action::CatalogEditorCancel,
        };
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(expected.clone())
        );
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), Some(expected));
        if matches!(
            action_field,
            lazydb::model::catalog_editor::TableActionField::AddColumn
                | lazydb::model::catalog_editor::TableActionField::RemoveColumn
        ) {
            assert_eq!(
                keymap.map(key(KeyCode::Esc), &app),
                Some(Action::CatalogEditorCancel)
            );
        }
    }
}

#[test]
fn table_editor_opens_column_details_with_e_but_not_enter() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

    assert_eq!(keymap.map(key(KeyCode::Enter), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::CatalogEditorOpenTableColumnDetails)
    );
}

#[test]
fn table_editor_column_details_uses_enter_to_confirm_and_esc_to_cancel() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.update(Action::CatalogEditorOpenTableColumnDetails);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name
        )
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::CatalogEditorInsert('n'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Backspace), &app),
        Some(Action::CatalogEditorBackspace)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorConfirmTableColumnDetails)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancelTableColumnDetails)
    );
}

#[test]
fn table_editor_column_details_keeps_navigation_inside_modal() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    app.update(Action::CatalogEditorOpenTableColumnDetails);
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );

    app.update(Action::CatalogEditorFieldNext);
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
}

#[test]
fn table_editor_columns_keymap_keeps_parent_navigation() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    app.update(Action::CatalogEditorFieldPrevious);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(draft.selected_column, 0);

    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    app.update(Action::CatalogEditorFieldNext);
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    app.update(Action::CatalogEditorFieldNext);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::Action(
            lazydb::model::catalog_editor::TableActionField::AddColumn
        )
    );
}

#[test]
fn table_editor_nullable_and_identity_keep_enter_and_space_toggle_semantics() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    for field in [
        lazydb::model::catalog_editor::TableColumnField::Nullable,
        lazydb::model::catalog_editor::TableColumnField::Identity,
    ] {
        let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        else {
            panic!("table draft expected");
        };
        draft.begin_edit_selected_column();
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(field);

        let expected = match field {
            lazydb::model::catalog_editor::TableColumnField::Nullable => {
                Action::CatalogEditorToggleTableColumnNullable
            }
            lazydb::model::catalog_editor::TableColumnField::Identity => {
                Action::CatalogEditorToggleTableColumnIdentity
            }
            _ => unreachable!(),
        };
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::CatalogEditorConfirmTableColumnDetails)
        );
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), Some(expected));
        app.update(Action::CatalogEditorCancelTableColumnDetails);
    }
}

#[test]
fn table_editor_action_buttons_keep_arrow_navigation() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Action(
        lazydb::model::catalog_editor::TableActionField::Review,
    );

    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
}

#[test]
fn constraint_editor_maps_field_navigation_and_text_input() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::CheckConstraint,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Constraint(
            lazydb::model::catalog_editor::ConstraintDraft::new(
                lazydb::db::catalog_mutation::ConstraintDefinitionKind::Check {
                    expression: String::new(),
                    no_inherit: false,
                },
                "public",
                "items",
            ),
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::CatalogEditorInsert('x'))
    );
}

#[test]
fn role_editor_uses_catalog_editor_field_keymap() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Role),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Role(
            lazydb::model::catalog_editor::RoleDraft::new(false),
        )),
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
    });
    app.overlay = Some(lazydb::model::workspace::Overlay::CatalogEditor);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
}

#[test]
fn data_query_completion_keys_preempt_query_input_navigation() {
    let mut app = App::new(Vec::new());
    let mut tab = RelationTab::new("users");
    tab.query.focus = Some(DataQueryInput::Where);
    tab.query.completion = Some(DataQueryCompletion {
        candidates: vec![DataQueryCandidate {
            name: "user_id".into(),
            type_name: Some("bigint".into()),
        }],
        selected: 0,
        replace: lazydb::sql::TextRange::new(0, 0),
    });
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(ctrl('n'), &app),
        Some(Action::DataQueryCompletionNext)
    );
    assert_eq!(
        keymap.map(ctrl('p'), &app),
        Some(Action::DataQueryCompletionPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::DataQueryCompletionAccept)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::DataQueryCompletionDismiss)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::DataQueryCompletionAccept)
    );

    let WorkspaceTab::Relation(tab) = &mut app.tabs[1] else {
        panic!()
    };
    tab.query.completion = None;
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::FocusDataQueryInput(DataQueryInput::OrderBy))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CancelDataQueryInput)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SubmitDataQuery)
    );
}

#[test]
fn help_overlay_owns_text_selection_and_execution_keys() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::HelpEdit(TextInputEdit::Insert('q')))
    );
    app.update(Action::HelpEdit(TextInputEdit::Insert('q')));
    assert_eq!(
        keymap.map(key(KeyCode::Backspace), &app),
        Some(Action::HelpEdit(TextInputEdit::Backspace))
    );
    app.update(Action::HelpEdit(TextInputEdit::Backspace));
    for (character, edit) in [
        ('w', TextInputEdit::DeletePreviousWord),
        ('u', TextInputEdit::Clear),
        ('a', TextInputEdit::MoveHome),
        ('e', TextInputEdit::MoveEnd),
    ] {
        assert_eq!(
            keymap.map(ctrl(character), &app),
            Some(Action::HelpEdit(edit))
        );
    }
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::HelpMove(1))
    );
    app.update(Action::HelpMove(1));
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::HelpMove(-1))
    );
    app.update(Action::HelpMove(-1));
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ExecuteHelpShortcut(
            lazydb::help::HelpShortcutId::Help
        ))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::DismissOverlay)
    );
}

#[test]
fn record_view_owns_navigation_keys_and_goto_sequence() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    app.overlay = Some(Overlay::RecordView(Default::default()));

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::RecordViewMoveFields(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::RecordViewMoveRow(1))
    );
    assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('g')), &app),
        Some(Action::RecordViewJumpFirstField)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::CloseRecordView)
    );
}

#[test]
fn record_view_pending_is_invalid_after_overlay_closes_outside_keymap() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    app.overlay = Some(Overlay::RecordView(Default::default()));

    assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
    app.update(Action::CloseRecordView);
    assert!(
        keymap
            .sequence_state(&app, std::time::Instant::now())
            .is_none()
    );
    assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
}

#[test]
fn help_overlay_accepts_pasted_search_text() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    assert_eq!(
        map_paste("ctrl\neditor".into(), &app),
        [Action::HelpPaste("ctrl\neditor".into())]
    );
}

#[test]
fn catalog_drop_overlay_maps_only_confirmation_keys_without_bypassing_text_entry() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::CatalogDropConfirm {
        plan: Box::new(
            lazydb::db::catalog_drop::CatalogDropPlan::new(
                lazydb::db::catalog_drop::CatalogDropRequest::new(
                    lazydb::identity::ConnectionIdentity {
                        profile_id: uuid::Uuid::nil(),
                        generation: 1,
                    },
                    lazydb::db::catalog::CatalogId::new(
                        uuid::Uuid::nil(),
                        lazydb::db::catalog::CatalogKind::Table,
                        ["app", "public", "users"],
                    ),
                    1,
                ),
                &lazydb::db::catalog::CatalogEntry::relation(
                    lazydb::db::catalog::CatalogId::new(
                        uuid::Uuid::nil(),
                        lazydb::db::catalog::CatalogKind::Table,
                        ["app", "public", "users"],
                    ),
                    lazydb::db::catalog::CatalogId::new(
                        uuid::Uuid::nil(),
                        lazydb::db::catalog::CatalogKind::Schema,
                        ["app", "public"],
                    ),
                    lazydb::db::catalog::QualifiedName {
                        database: Some("app".into()),
                        schema: Some("public".into()),
                        object: "users".into(),
                    },
                    "table",
                    lazydb::db::catalog::OptionalMetadata::Unsupported,
                    true,
                )
                .unwrap(),
                "DROP TABLE users",
            )
            .unwrap(),
        ),
        input: Default::default(),
        busy: false,
        error: None,
    });
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogDropConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::CatalogDropInsert('y'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('Y')), &app),
        Some(Action::CatalogDropInsert('Y'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogDropCancel)
    );
}

#[test]
fn catalog_editor_picker_and_preview_own_their_keys() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: lazydb::db::catalog_mutation::CatalogObjectType::LoginRole,
            label: "Login Role".into(),
        }],
    ));
    app.overlay = Some(Overlay::CatalogEditor);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorSelect)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancel)
    );

    app.catalog_editor.as_mut().unwrap().page =
        lazydb::model::catalog_editor::CatalogEditorPage::SqlPreview;
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorApply)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorBack)
    );
}

#[test]
fn table_column_details_escape_leaves_every_field_without_text_capture() {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Group {
            schema: lazydb::db::catalog::CatalogId::new(
                Uuid::nil(),
                lazydb::db::catalog::CatalogKind::Schema,
                ["app", "public"],
            ),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Table,
            ),
            label: "Table".into(),
        }],
    );
    assert!(
        editor.select_object_type(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Table,
        ))
    );
    app.catalog_editor = Some(editor);
    app.overlay = Some(Overlay::CatalogEditor);
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.begin_edit_selected_column();
    }
    let mut keymap = Keymap::default();

    for field in [
        lazydb::model::catalog_editor::TableColumnField::Name,
        lazydb::model::catalog_editor::TableColumnField::Comment,
        lazydb::model::catalog_editor::TableColumnField::Identity,
    ] {
        let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        else {
            panic!("table draft expected");
        };
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(field);
        assert_eq!(
            keymap.map(key(KeyCode::Esc), &app),
            Some(Action::CatalogEditorCancelTableColumnDetails)
        );
    }

    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.cancel_column_details();
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::General(
            lazydb::model::catalog_editor::TableGeneralField::Name,
        );
    } else {
        panic!("table draft expected");
    }
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancel)
    );
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    } else {
        panic!("table draft expected");
    }
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancel)
    );
}

#[test]
fn explorer_catalog_shortcuts_are_not_advertised_for_synthetic_rows() {
    let profile = profile("sqlite");
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenExplorerAdd)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::OpenCatalogEdit)
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Status {
        owner: lazydb::model::explorer::ExplorerOwnerId::Profile(profile.id),
        kind: lazydb::model::explorer::StatusRowKind::Loading,
    });
    assert_eq!(keymap.map(key(KeyCode::Char('a')), &app), None);
}

fn owner_choice(name: &str, is_current: bool) -> lazydb::db::catalog_mutation::CatalogOwnerChoice {
    lazydb::db::catalog_mutation::CatalogOwnerChoice {
        name: name.into(),
        can_login: true,
        selectable: true,
        is_current,
    }
}

/// Connected postgres profile with a discovered owner list and a schema form on the name row.
fn schema_editor_with_owner_choices() -> App {
    let profile_id = Uuid::from_u128(0x5c8e);
    let mut app = App::new(Vec::new());
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.owner_context = lazydb::model::workspace::CatalogOwnerContextState::Loaded {
        connection: lazydb::model::workspace::ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        context: lazydb::db::catalog_mutation::CatalogOwnerContext {
            current_user: "postgres".into(),
            choices: vec![
                owner_choice("app_owner", false),
                owner_choice("postgres", true),
            ],
        },
    };
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile { profile_id },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Schema,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Schema(
            lazydb::model::catalog_editor::SchemaDraft {
                name: "sales".into(),
                owner: "postgres".into(),
                comment: "".into(),
                selected_field: 0,
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    app
}

fn schema_draft(app: &App) -> &lazydb::model::catalog_editor::SchemaDraft {
    match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    {
        Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) => draft,
        _ => panic!("schema draft expected"),
    }
}

#[test]
fn schema_owner_picker_hands_navigation_back_to_the_form() {
    let mut app = schema_editor_with_owner_choices();
    let mut keymap = Keymap::default();

    let action = keymap.map(key(KeyCode::Tab), &app).expect("tab into owner");
    assert_eq!(action, Action::CatalogEditorFieldNext);
    app.update(action);
    assert!(
        app.catalog_editor
            .as_ref()
            .expect("editor")
            .owner_picker_active()
    );

    let action = keymap.map(key(KeyCode::Down), &app).expect("select role");
    assert_eq!(action, Action::CatalogOwnerPickerMove(1));
    app.update(action);
    let action = keymap.map(key(KeyCode::Enter), &app).expect("accept role");
    assert_eq!(action, Action::CatalogOwnerPickerAccept);
    app.update(action);
    assert_eq!(schema_draft(&app).owner.value(), "app_owner");
    assert!(
        !app.catalog_editor
            .as_ref()
            .expect("editor")
            .owner_picker
            .open
    );

    // Accepting a role must not swallow field navigation.
    let action = keymap
        .map(key(KeyCode::Tab), &app)
        .expect("tab after accepting a role");
    assert_eq!(action, Action::CatalogEditorFieldNext);
    app.update(action);
    assert_eq!(schema_draft(&app).selected_field, 2);

    let action = keymap
        .map(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &app)
        .expect("shift-tab back to owner");
    assert_eq!(action, Action::CatalogEditorFieldPrevious);
    app.update(action);
    assert!(
        app.catalog_editor
            .as_ref()
            .expect("editor")
            .owner_picker_active()
    );

    let action = keymap.map(key(KeyCode::Esc), &app).expect("close the list");
    assert_eq!(action, Action::CatalogOwnerPickerClose);
    app.update(action);
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancel)
    );
}

#[test]
fn leaving_the_owner_row_releases_the_owner_list() {
    let mut app = schema_editor_with_owner_choices();
    let mut keymap = Keymap::default();

    app.update(Action::CatalogEditorFocusField(1));
    assert!(
        app.catalog_editor
            .as_ref()
            .expect("editor")
            .owner_picker_active()
    );

    app.update(Action::CatalogEditorFocusField(2));
    assert!(
        !app.catalog_editor
            .as_ref()
            .expect("editor")
            .owner_picker
            .open
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::CatalogEditorInsert('x'))
    );
}

#[test]
fn view_editor_form_owns_navigation_and_preview_keys() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::View,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::View(
            lazydb::model::catalog_editor::ViewDraft {
                name: "v".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                query: "SELECT 1".into(),
                output_columns: "".into(),
                security_barrier: lazydb::db::catalog_mutation::ViewOption::unavailable("test"),
                security_invoker: lazydb::db::catalog_mutation::ViewOption::unavailable("test"),
                check_option: lazydb::db::catalog_mutation::ViewOption::unavailable("test"),
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::CatalogEditorInsert('x'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
}

#[test]
fn sequence_editor_form_owns_text_input_keys() {
    let mut app = App::new(vec![
        import_connection_url(":memory:", Some("test"))
            .unwrap()
            .profile,
    ]);
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Sequence(
            lazydb::model::catalog_editor::SequenceDraft {
                name: "seq".into(),
                schema: "public".into(),
                owner: "owner".into(),
                comment: "".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: lazydb::db::catalog_mutation::SequenceBound::Unset,
                max_value: lazydb::db::catalog_mutation::SequenceBound::Unset,
                start_value: "1".into(),
                restart_value: "".into(),
                cache: "1".into(),
                cycle: false,
                owned_by: "NONE".into(),
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
            },
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        ..Default::default()
    });
    app.overlay = Some(lazydb::model::workspace::Overlay::CatalogEditor);
    let mut keymap = Keymap::default();
    assert!(matches!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::CatalogEditorInsert('x'))
    ));
}

#[test]
fn filtered_help_moves_to_non_first_id_and_executes_it() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.focus = Focus::Editor;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("move focus".into()));
    app.update(Action::HelpMove(1));

    let mut keymap = Keymap::default();
    let action = keymap.map(key(KeyCode::Enter), &app);
    assert_eq!(
        action,
        Some(Action::ExecuteHelpShortcut(
            lazydb::help::HelpShortcutId::FocusResults
        ))
    );
    app.update(action.unwrap());

    assert_eq!(app.focus, Focus::Results);
    assert_eq!(app.overlay, None);
}

fn profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

fn window_action(app: &App, direction: char) -> Option<Action> {
    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(ctrl('w'), app), None);
    keymap.map(key(KeyCode::Char(direction)), app)
}

#[test]
fn ctrl_w_ctrl_w_maps_to_focus_next_outside_editor() {
    let mut app = App::new(Vec::new());

    for focus in [Focus::Explorer, Focus::Results] {
        app.focus = focus;
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(ctrl('w'), &app), None);
        assert_eq!(keymap.map(ctrl('w'), &app), Some(Action::FocusNext));
    }
}

#[test]
fn ctrl_w_plain_w_does_not_cycle_focus() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('w')), &app), None);
}

fn apply_key(keymap: &mut Keymap, app: &mut App, event: KeyEvent) {
    if let Some(action) = keymap.map(event, app) {
        app.update(action);
    }
}

fn cycle_focus(keymap: &mut Keymap, app: &mut App) {
    apply_key(keymap, app, ctrl('w'));
    apply_key(keymap, app, ctrl('w'));
}

#[test]
fn ctrl_w_ctrl_w_cycles_sql_panes_clockwise() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Editor);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Results);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Explorer);
}

#[test]
fn ctrl_w_ctrl_w_cycles_relation_panes_without_editor() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Results);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Explorer);
}

#[test]
fn ctrl_w_ctrl_w_cycles_dashboard_panes_without_editor() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::Dashboard(
        lazydb::model::dashboard::DashboardTab::new(),
    ));
    app.active_tab = 0;
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Results);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Explorer);
}

fn counted_window_action(app: &App, count: &str, operator: char) -> Option<Action> {
    let mut keymap = Keymap::default();
    for character in count.chars() {
        assert_eq!(keymap.map(key(KeyCode::Char(character)), app), None);
    }
    assert_eq!(keymap.map(ctrl('w'), app), None);
    keymap.map(key(KeyCode::Char(operator)), app)
}

#[test]
fn relation_window_directions_target_existing_panes() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;

    assert_eq!(window_action(&app, 'h'), None);
    assert_eq!(window_action(&app, 'j'), None);
    assert_eq!(window_action(&app, 'k'), None);
    assert_eq!(
        window_action(&app, 'l'),
        Some(Action::Focus(Focus::Results))
    );

    app.focus = Focus::Results;
    assert_eq!(
        window_action(&app, 'h'),
        Some(Action::Focus(Focus::Explorer))
    );
    assert_eq!(window_action(&app, 'k'), None);
}

#[test]
fn sql_window_directions_keep_three_pane_mapping() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(window_action(&app, 'h'), None);
    assert_eq!(window_action(&app, 'j'), None);
    assert_eq!(window_action(&app, 'k'), None);
    assert_eq!(window_action(&app, 'l'), Some(Action::Focus(Focus::Editor)));

    app.focus = Focus::Results;
    assert_eq!(window_action(&app, 'k'), Some(Action::Focus(Focus::Editor)));
}

#[test]
fn output_o_toggles_back_to_data_instead_of_entering_read_only_vim_open_line() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    app.active_console_mut().result_view = lazydb::model::tab::ResultView::Output;

    assert_eq!(
        Keymap::default().map(key(KeyCode::Char('o')), &app),
        Some(Action::ToggleResultView)
    );
}

#[test]
fn editor_space_uses_editor_binding_path_not_application_leader_pending() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();

    assert!(matches!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(_))
    ));
}

#[test]
fn editor_normal_space_exposes_editor_leader_candidates_without_consuming_next_key() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();

    assert!(matches!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(_))
    ));
    let state = keymap
        .sequence_state(&app, std::time::Instant::now())
        .expect("editor leader state");
    assert_eq!(state.prefix, lazydb::help::ShortcutPrefix::EditorLeader);
    assert!(matches!(
        keymap.map(key(KeyCode::Char('f')), &app),
        Some(Action::EditorKey(_))
    ));
    assert!(
        keymap
            .sequence_state(&app, std::time::Instant::now())
            .is_none()
    );
}

#[test]
fn editor_space_tt_reaches_editor_transaction_toggle_binding() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();

    assert!(matches!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(_))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::EditorKey(_))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::EditorKey(_))
    ));
}

#[test]
fn editor_leader_followups_stay_on_the_editor_modalkit_path() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));

    for character in ['f', 'y', 'Y', 'd'] {
        let mut keymap = Keymap::default();
        assert!(matches!(
            keymap.map(key(KeyCode::Char(' ')), &app),
            Some(Action::EditorKey(_))
        ));
        assert!(matches!(
            keymap.map(key(KeyCode::Char(character)), &app),
            Some(Action::EditorKey(_))
        ));
        assert!(
            keymap
                .sequence_state(&app, std::time::Instant::now())
                .is_none()
        );
    }
}

#[test]
fn data_query_completion_controls_match_catalog_context() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().query.focus = Some(DataQueryInput::Where);
    app.active_console_mut().query.capability = lazydb::model::data_query::DataQueryCapability::Sql;
    app.active_console_mut().query.completion = Some(DataQueryCompletion {
        candidates: vec![DataQueryCandidate {
            name: "active".into(),
            type_name: Some("BOOLEAN".into()),
        }],
        selected: 0,
        replace: lazydb::sql::TextRange::new(0, 0),
    });
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(ctrl('n'), &app),
        Some(Action::DataQueryCompletionNext)
    );
    assert_eq!(
        keymap.map(ctrl('p'), &app),
        Some(Action::DataQueryCompletionPrevious)
    );
}

#[test]
fn relation_browse_yy_maps_the_catalog_yank_row_binding() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char('y')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::RelationYank)
    );
}

#[test]
fn sql_help_window_directions_match_three_pane_mapping() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("move focus".into()));
    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::FocusEditorFromL)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::FocusEditorFromL,
    ));
    assert_eq!(app.focus, Focus::Editor);

    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("move focus".into()));
    let rows = [app.help_selected_id()];
    assert_eq!(rows, [Some(lazydb::help::HelpShortcutId::FocusExplorer)]);
}

#[test]
fn relation_help_window_directions_match_two_pane_mapping() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("move focus".into()));

    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::FocusResultsFromL)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::FocusResultsFromL,
    ));
    assert_eq!(app.focus, Focus::Results);
    assert_eq!(app.overlay, None);
}

#[test]
fn maps_counted_pane_resize_commands() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(window_action(&app, '+'), None);
    assert_eq!(
        window_action(&app, '>'),
        Some(Action::ResizePane(lazydb::model::workspace::PaneResize {
            split: lazydb::model::workspace::PaneSplit::ExplorerWidth,
            delta: 1,
        },))
    );
    assert_eq!(
        counted_window_action(&app, "10", '>'),
        Some(Action::ResizePane(lazydb::model::workspace::PaneResize {
            split: lazydb::model::workspace::PaneSplit::ExplorerWidth,
            delta: 10,
        }))
    );
    app.focus = Focus::Results;
    assert_eq!(
        counted_window_action(&app, "12", '-'),
        Some(Action::ResizePane(lazydb::model::workspace::PaneResize {
            split: lazydb::model::workspace::PaneSplit::EditorHeight,
            delta: 12,
        }))
    );
    assert_eq!(window_action(&app, '='), Some(Action::ResetPaneSizes));
}

#[test]
fn maps_window_f_to_toggle_focused_pane_maximize() {
    let mut app = App::new(Vec::new());

    for focus in [Focus::Explorer, Focus::Results] {
        app.focus = focus;
        assert_eq!(window_action(&app, 'f'), Some(Action::TogglePaneMaximized));
    }
}

#[test]
fn dashboard_results_focus_maps_window_f_to_toggle_focused_pane_maximize() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::Dashboard(
        lazydb::model::dashboard::DashboardTab::new(),
    ));
    app.active_tab = 0;
    app.focus = Focus::Results;

    assert_eq!(window_action(&app, 'f'), Some(Action::TogglePaneMaximized));
}

#[test]
fn dashboard_results_focus_executes_window_f_to_toggle_focused_pane_maximize() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::Dashboard(
        lazydb::model::dashboard::DashboardTab::new(),
    ));
    app.active_tab = 0;
    app.focus = Focus::Results;
    let action = window_action(&app, 'f').expect("dashboard Ctrl-w f action");

    app.update(action);

    assert!(app.pane_maximized);
}

#[test]
fn dashboard_window_f_works_on_each_page_and_focus() {
    for page in [
        lazydb::model::dashboard::DashboardPage::Overview,
        lazydb::model::dashboard::DashboardPage::Processes,
    ] {
        for focus in [Focus::Explorer, Focus::Results] {
            let mut app = App::new(Vec::new());
            app.tabs.clear();
            let mut dashboard = lazydb::model::dashboard::DashboardTab::new();
            dashboard.page = page;
            app.tabs.push(WorkspaceTab::Dashboard(dashboard));
            app.active_tab = 0;
            app.focus = focus;

            let action = window_action(&app, 'f').expect("dashboard Ctrl-w f action");
            app.update(action);

            assert!(app.pane_maximized, "page={page:?} focus={focus:?}");
        }
    }
}

#[test]
fn pane_maximize_help_entry_executes_the_same_action() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("maximize or restore focused pane".into()));

    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::TogglePaneMaximized)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::TogglePaneMaximized,
    ));

    assert!(app.pane_maximized);
    assert_eq!(app.overlay, None);
}

#[test]
fn cycle_focus_help_entry_executes_the_same_action() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("cycle pane focus clockwise".into()));

    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::CyclePaneFocus)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::CyclePaneFocus,
    ));

    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.overlay, None);
}

#[test]
fn maps_global_sequences_and_function_keys() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(key(KeyCode::Char('?')), &app),
        Some(Action::ShowHelp)
    );
    assert_eq!(
        keymap.map(key(KeyCode::F(5)), &app),
        Some(Action::RunActiveSql)
    );
    assert_eq!(keymap.map(key(KeyCode::F(1)), &app), Some(Action::ShowHelp));
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('h')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Esc), &app), None);
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(ctrl('h'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Esc), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char(']')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::NextTab)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('n')), &app), None);
    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenSqlEditorList)
    );
    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::CloseActiveTab)
    );
}

#[test]
fn maps_sql_editor_delete_and_list_shortcuts() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::RequestDeleteActiveConsole)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('e')), &app), None);
    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenSqlEditorList)
    );
}

#[test]
fn maps_tab_sequences_from_editor_normal_mode() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char('[')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::PreviousTab)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(']')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::NextTab)
    );

    assert!(matches!(
        keymap.map(key(KeyCode::Char('g')), &app),
        Some(Action::EditorKey(_))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('T')), &app),
        Some(Action::EditorKey(_))
    ));
    assert_eq!(
        keymap.map(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL),
            &app,
        ),
        Some(Action::NextTab)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL), &app,),
        Some(Action::PreviousTab)
    );
}

#[test]
fn maps_column_details_navigation_and_toggle_keys() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(
            lazydb::model::catalog_editor::TableDraft::new("public"),
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.begin_edit_selected_column();
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Nullable,
        );
    }
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorConfirmTableColumnDetails)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogEditorCancelTableColumnDetails)
    );
}

#[test]
fn numeric_result_keys_start_window_counts_without_stealing_query_input() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char('1')), &app), None);
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('>')), &app),
        Some(Action::ResizePane(lazydb::model::workspace::PaneResize {
            split: lazydb::model::workspace::PaneSplit::ExplorerWidth,
            delta: -1,
        }))
    );

    let mut relation = RelationTab::new("users");
    relation.query.focus = Some(DataQueryInput::Where);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    assert_eq!(
        keymap.map(key(KeyCode::Char('1')), &app),
        Some(Action::DataQueryInsert('1'))
    );

    app.tabs[1] = WorkspaceTab::Relation(RelationTab::new("users"));
    assert_eq!(keymap.map(key(KeyCode::Char('1')), &app), None);
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('<')), &app),
        Some(Action::ResizePane(lazydb::model::workspace::PaneResize {
            split: lazydb::model::workspace::PaneSplit::ExplorerWidth,
            delta: 1,
        }))
    );
}

#[test]
fn space_s_opens_console_manager_from_explorer() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 2;
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenSqlEditorList)
    );
}

#[test]
fn space_s_opens_console_manager_from_sql_results() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenSqlEditorList)
    );
}

#[test]
fn shifted_y_copies_sql_result_rows() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let shifted_y = KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT);

    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(shifted_y, &app),
        Some(Action::CopyGridRow {
            include_headers: false,
        })
    );

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(shifted_y, &app),
        Some(Action::CopyGridRow {
            include_headers: true,
        })
    );

    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::Char('Y'), modifiers), &app),
            None
        );
    }
}

#[test]
fn maps_scope_picker_navigation_and_toggle() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    {
        let draft = app
            .profile_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap();
        draft.name.set("scope");
        draft.database.set("lazydb");
    }
    app.update(Action::ProfileOpenScope);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::ProfileScopeMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ProfileScopeMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::ProfileScopeMove(-1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::ProfileScopeMove(-1))
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('r')), &app), None);
}

#[test]
fn maps_explicit_full_buffer_execution_separately() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::F(5), KeyModifiers::SHIFT), &app,),
        Some(Action::RunAllSql)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('R')), &app),
        Some(Action::RunAllSql)
    );
}

#[test]
fn explorer_catalog_mutation_maps_selected_profile_root_actions_by_stable_id() {
    let profile = profile("root");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::OpenCatalogEdit)
    );
    app.update(Action::OpenCatalogEdit);
    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));
    app.update(Action::CloseProfileManager);
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::ProfileStartNew)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::ProfileRequestDelete { profile_id })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::RequestProfileDisconnect { profile_id })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ExplorerOpenSelected)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::ExplorerToggle)
    );

    app.focus = Focus::Explorer;
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::ExplorerToggle)
    );
}

#[test]
fn explorer_catalog_mutation_maps_catalog_node_without_profile_fallback() {
    let profile = profile("owner");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let node_id = lazydb::db::catalog::CatalogId::new(
        profile_id,
        lazydb::db::catalog::CatalogKind::Database,
        ["app"],
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(node_id.clone()));
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char('e')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('a')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::RequestDropCatalogObject {
            id: node_id.clone()
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::RequestProfileConnect { profile_id })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::RequestProfileDisconnect { profile_id })
    );
}

#[test]
fn explorer_catalog_mutation_synthetic_nodes_are_noops() {
    let profile = profile("synthetic");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    for selected in [
        ExplorerNodeId::EmptyProfiles,
        ExplorerNodeId::Others,
        ExplorerNodeId::Status {
            owner: lazydb::model::explorer::ExplorerOwnerId::Profile(profile_id),
            kind: lazydb::model::explorer::StatusRowKind::Loading,
        },
        ExplorerNodeId::Empty {
            owner: lazydb::model::explorer::ExplorerOwnerId::Profile(profile_id),
        },
    ] {
        app.explorer.normalized.selected = Some(selected);
        assert_eq!(keymap.map(key(KeyCode::Char('a')), &app), None);
        assert_eq!(keymap.map(key(KeyCode::Char('e')), &app), None);
    }
}

fn materialized_view_editor(
    mode: lazydb::db::catalog_mutation::CatalogMutationMode,
    focus: lazydb::model::catalog_editor::CatalogFormFocus,
) -> App {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::MaterializedView,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        draft: Some(
            lazydb::model::catalog_editor::CatalogDraft::MaterializedView(
                lazydb::model::catalog_editor::MaterializedViewDraft {
                    name: "mv".into(),
                    schema: "public".into(),
                    owner: "postgres".into(),
                    comment: "".into(),
                    query: "SELECT 1".into(),
                    tablespace: "".into(),
                    with_data: true,
                    focus,
                    query_editable: mode
                        == lazydb::db::catalog_mutation::CatalogMutationMode::Create,
                },
            ),
        ),
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
    });
    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.overlay = Some(Overlay::CatalogEditor);
    app
}

#[test]
fn materialized_view_tablespace_maps_space_to_text_input() {
    let app = materialized_view_editor(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::model::catalog_editor::CatalogFormFocus::Tablespace,
    );
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::CatalogEditorInsert(' '))
    );
}

#[test]
fn materialized_view_data_focus_maps_space_to_toggle() {
    let app = materialized_view_editor(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::model::catalog_editor::CatalogFormFocus::WithData,
    );
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::CatalogEditorToggleMaterializedViewData)
    );
}

#[test]
fn insert_mode_preserves_printable_characters() {
    let mut keymap = Keymap::default();
    let app = App::new(Vec::new());
    assert_eq!(app.active_editor_mode(), EditorMode::Insert);

    assert_eq!(
        keymap.map(key(KeyCode::Char('?')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('?'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('q'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::EditorKey(key(KeyCode::Esc)))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::EditorKey(key(KeyCode::Tab)))
    );
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::Quit));
}

#[test]
fn normal_mode_global_keys_win_over_editor_and_completion() {
    for popup in [false, true] {
        let mut keymap = Keymap::default();
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        if popup {
            app.active_console_mut().completion = Some(CompletionPopup::default());
        }

        assert_eq!(
            keymap.map(key(KeyCode::Char('?')), &app),
            Some(Action::ShowHelp)
        );
        assert_eq!(keymap.map(key(KeyCode::Tab), &app), Some(Action::FocusNext));
        assert_eq!(
            keymap.map(key(KeyCode::BackTab), &app),
            Some(Action::FocusPrevious)
        );
    }
}

#[test]
fn shift_tab_focuses_the_previous_pane_in_both_terminal_encodings() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), &app,),
        Some(Action::FocusPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::FocusPrevious)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &app,),
        Some(Action::FocusPrevious)
    );
}

#[test]
fn shift_tab_leaves_sql_insert_mode_and_focuses_the_previous_pane() {
    let mut keymap = Keymap::default();
    let app = App::new(Vec::new());
    assert_eq!(app.active_editor_mode(), EditorMode::Insert);

    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), &app),
        Some(Action::FocusPrevious)
    );
}

#[test]
fn insert_escape_bypasses_completion_dismiss() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.active_console_mut().completion = Some(CompletionPopup::default());
    let escape = key(KeyCode::Esc);
    assert_eq!(keymap.map(escape, &app), Some(Action::EditorKey(escape)));
}

#[test]
fn printable_input_passes_through_an_open_completion_popup() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.active_console_mut().completion = Some(CompletionPopup::default());

    let event = key(KeyCode::Char('e'));
    assert_eq!(keymap.map(event, &app), Some(Action::EditorKey(event)));
    assert_eq!(keymap.map(ctrl('n'), &app), Some(Action::CompletionNext));
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CompletionAccept)
    );
}

#[test]
fn normal_mode_does_not_route_keys_to_a_stale_completion_popup() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.active_console_mut().completion = Some(CompletionPopup::default());

    assert_eq!(keymap.map(ctrl('n'), &app), Some(Action::NextTab));
    assert_eq!(keymap.map(ctrl('p'), &app), Some(Action::PreviousTab));
    assert_ne!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CompletionAccept)
    );
}

#[test]
fn transaction_exit_keys_carry_their_explicit_choice() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    let prompt = lazydb::model::transaction::DeferredTransactionPrompt {
        console_id: app.active_console().id,
        transaction_generation: 0,
        intent: lazydb::model::transaction::DeferredIntent::CloseConsole,
    };
    app.overlay = Some(lazydb::model::workspace::Overlay::TransactionExitConfirm {
        prompt,
        choice: lazydb::model::transaction::TransactionExitChoice::Rollback,
    });

    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::ConfirmTransactionExitChoice(
            lazydb::model::transaction::TransactionExitChoice::Rollback,
        ))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::ConfirmTransactionExitChoice(
            lazydb::model::transaction::TransactionExitChoice::Commit,
        ))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ConfirmTransactionExit)
    );
}

#[test]
fn relation_transaction_control_uses_space_tc_but_keeps_ctrl_commit_rollback() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = lazydb::model::workspace::Focus::Results;

    assert_eq!(
        keymap.map(
            crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL,),
            &app,
        ),
        Some(Action::RelationCommit)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::OpenTransactionControl)
    );
}

#[test]
fn relation_help_executes_space_tc_transaction_control() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = lazydb::model::transaction::TransactionState::Active;
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;

    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("commit or roll back transaction".into()));
    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::TransactionControl)
    );

    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::TransactionControl,
    ));
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::RelationTransactionConfirm { .. })
    ));
}

#[test]
fn editor_leader_opens_connection_target_selector() {
    let mut keymap = Keymap::default();
    let profile = profile("target");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.update(Action::EditorKey(key(KeyCode::Char(' '))));
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('d'))))
    );
    app.update(Action::EditorKey(key(KeyCode::Char('d'))));
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::TargetSelector {
            ref candidates,
            selected: 0,
        }) if candidates.len() == 1 && candidates[0].profile_id == profile_id
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CancelTargetSelector)
    );
}

#[test]
fn maps_vim_editor_navigation_in_normal_mode() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));

    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('h'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('j'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('k'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('l'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('i')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('i'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('a'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('o'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(key(KeyCode::Char(' '))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('r'))))
    );
}

#[test]
fn maps_explorer_and_result_actions_by_context() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ExplorerMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::ExplorerExpand)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('p')), &app),
        Some(Action::OpenSelectedRelation {
            view: lazydb::model::relation::RelationView::Data,
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('D')), &app),
        Some(Action::OpenSelectedRelation {
            view: lazydb::model::relation::RelationView::Ddl,
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::ExplorerRefresh)
    );

    app.focus = Focus::Results;
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::GridMove {
            rows: 1,
            columns: 0
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::ToggleResultView)
    );

    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::ExplorerToggle)
    );

    app.focus = Focus::Results;
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::SetRelationView(
            lazydb::model::relation::RelationView::Ddl
        ))
    );
    app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::SetRelationView(
            lazydb::model::relation::RelationView::Data
        ))
    );
}

#[test]
fn explorer_a_on_a_profile_opens_connection_group_creation() {
    let profile = lazydb::profile::import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenExplorerAdd)
    );
}

#[test]
fn explorer_add_overlay_owns_navigation_and_cancel_keys() {
    let profile = lazydb::profile::import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.update(Action::OpenExplorerAdd);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ExplorerAddMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::ExplorerAddMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::ExplorerAddMove(-1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::ExplorerAddMove(-1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ExplorerAddConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::ExplorerAddCancel)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::ExplorerAddCancel)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('a')), &app), None);
}

#[test]
fn explorer_g_prefix_lists_and_opens_move_to_group() {
    let profile = lazydb::profile::import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
    assert!(
        keymap
            .sequence_state(&app, std::time::Instant::now())
            .is_some()
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('m')), &app),
        Some(Action::ProfileGroupOpen)
    );

    for (suffix, expected) in [
        (
            'g',
            Action::ExplorerSelectTarget(lazydb::model::explorer::ExplorerNodeTarget::First),
        ),
        ('t', Action::NextTab),
        ('T', Action::PreviousTab),
    ] {
        let mut keymap = Keymap::default();
        assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
        assert_eq!(keymap.map(key(KeyCode::Char(suffix)), &app), Some(expected));
    }
}

#[test]
fn profile_group_editor_routes_j_and_k_to_group_name_input() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::ProfileGroup(
        lazydb::model::profile_group::ProfileGroupOverlay::Edit {
            group_id: None,
            name: Default::default(),
            error: None,
            busy: false,
        },
    ));
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ProfileGroupEdit(TextInputEdit::Insert('j')))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::ProfileGroupEdit(TextInputEdit::Insert('k')))
    );
    for (character, edit) in [
        ('w', TextInputEdit::DeletePreviousWord),
        ('u', TextInputEdit::Clear),
        ('a', TextInputEdit::MoveHome),
        ('e', TextInputEdit::MoveEnd),
    ] {
        assert_eq!(
            keymap.map(ctrl(character), &app),
            Some(Action::ProfileGroupEdit(edit))
        );
    }
}

#[test]
fn confirmed_explorer_find_keeps_navigation_keys_available() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_find();
    app.explorer.confirm_find();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::ExplorerFindNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('N')), &app),
        Some(Action::ExplorerFindPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ExplorerMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::ExplorerMove(-1))
    );
    assert_eq!(
        keymap.map(ctrl('d'), &app),
        Some(Action::ExplorerScrollNodes {
            direction: 1,
            amount: lazydb::model::explorer::ExplorerScrollAmount::HalfPage,
        })
    );
    assert_eq!(
        keymap.map(ctrl('u'), &app),
        Some(Action::ExplorerScrollNodes {
            direction: -1,
            amount: lazydb::model::explorer::ExplorerScrollAmount::HalfPage,
        })
    );
}

#[test]
fn relation_keys_control_only_the_active_relation_view() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    // Relation Data reserves p for paste; the view shortcut remains available
    // when the DDL view is active.
    assert_eq!(
        keymap.map(key(KeyCode::Char('p')), &app),
        Some(Action::RelationPaste)
    );
    app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Char('p')), &app),
        Some(Action::SetRelationView(
            lazydb::model::relation::RelationView::Data
        ))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('D')), &app),
        Some(Action::SetRelationView(
            lazydb::model::relation::RelationView::Ddl
        ))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::RefreshActiveRelation)
    );
}

#[test]
fn page_size_selector_owns_navigation_and_apply_lifecycle() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('P')), &app),
        Some(Action::OpenPageSizeSelector { relation: false })
    );
    app.update(Action::OpenPageSizeSelector { relation: false });
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::MovePageSizeSelector(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ConfirmPageSizeSelector)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CancelPageSizeSelector)
    );
}

#[test]
fn ddl_view_maps_navigation_without_using_grid_move() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Results;
    app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));
    let mut keymap = Keymap::default();

    let session_id = match app.tabs.get(app.active_tab) {
        Some(lazydb::model::tab::WorkspaceTab::Relation(tab)) => tab.ddl_editor_id,
        _ => panic!("expected relation tab"),
    };
    for code in [
        KeyCode::Char('j'),
        KeyCode::Down,
        KeyCode::Char('k'),
        KeyCode::Up,
        KeyCode::Char('h'),
        KeyCode::Left,
        KeyCode::Char('l'),
        KeyCode::Right,
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char('V'),
    ] {
        assert_eq!(
            keymap.map(key(code), &app),
            Some(Action::ReadOnlyEditorKey {
                session_id,
                event: key(code),
            }),
            "code={code:?}"
        );
    }
}

#[test]
fn ctrl_c_is_the_global_exit_and_q_is_not() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;

    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('q'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('Q')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('Q'))))
    );
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::Quit));

    app.active_console_mut().completion = Some(CompletionPopup::default());
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::Quit));

    app.update(Action::EditorKey(key(KeyCode::Esc)));
    assert_eq!(
        keymap.map(key(KeyCode::Char('Q')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('Q'))))
    );
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::Quit));

    app.focus = Focus::Explorer;
    assert_eq!(keymap.map(key(KeyCode::Char('Q')), &app), None);
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::Quit));
}

#[test]
fn space_c_focuses_explorer_from_every_normal_mode_focus() {
    for focus in [Focus::Explorer, Focus::Editor, Focus::Results] {
        let mut app = App::new(Vec::new());
        app.focus = focus;
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char(' ')), &app),
            if focus == Focus::Editor {
                Some(Action::EditorKey(key(KeyCode::Char(' '))))
            } else {
                None
            }
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('c')), &app),
            if focus == Focus::Editor {
                Some(Action::EditorKey(key(KeyCode::Char('c'))))
            } else {
                Some(Action::Focus(Focus::Explorer))
            }
        );
    }

    let mut app = App::new(Vec::new());
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(key(KeyCode::Char(' '))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('c'))))
    );

    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.focus = Focus::Explorer;
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Char('i'))));
    assert_eq!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('c'))))
    );

    app.focus = Focus::Results;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.active_console_mut().query_status = QueryStatus::Running;
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(ctrl('c'), &app), None);

    app.active_console_mut().query_status = QueryStatus::Idle;
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let first_tab = app.active_tab;
    app.update(Action::NewConsole);
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let second_tab = app.active_tab;
    app.update(Action::ActivateTab(first_tab));
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(key(KeyCode::Char(' '))))
    );
    app.update(Action::ActivateTab(second_tab));
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('r'))))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::EditorKey(key(KeyCode::Char(' '))))
    );
    keymap.clear_pending();
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::EditorKey(key(KeyCode::Char('r'))))
    );
}

#[test]
fn profile_form_overlay_routes_before_generic_dismissal() {
    let mut app = App::new(vec![profile("first"), profile("second")]);
    app.update(Action::OpenProfileManager);
    let mut keymap = Keymap::default();

    let mappings = [
        (KeyCode::Tab, Action::ProfileFieldNext),
        (KeyCode::BackTab, Action::ProfileFieldPrevious),
        (KeyCode::Esc, Action::CloseProfileManager),
    ];
    for (code, expected) in mappings {
        assert_eq!(keymap.map(key(code), &app), Some(expected));
    }
    assert_eq!(keymap.map(ctrl('d'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('?')), &app), None);

    app.overlay = Some(Overlay::Help(lazydb::help::HelpState::new(
        lazydb::help::ShortcutContext::EditorNormal,
        lazydb::help::ShortcutCapabilities::default(),
    )));
    assert_eq!(
        keymap.map(key(KeyCode::Char('?')), &app),
        Some(Action::HelpEdit(TextInputEdit::Insert('?')))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::HelpEdit(TextInputEdit::Insert('q')))
    );
    assert_eq!(
        keymap.map(
            KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release,),
            &app,
        ),
        None
    );
}

#[test]
fn profile_form_maps_navigation_editing_and_commands() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::ProfileFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::BackTab), &app),
        Some(Action::ProfileFieldPrevious)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), &app),
        Some(Action::ProfileFieldPrevious)
    );
    assert_eq!(keymap.map(ctrl('t'), &app), Some(Action::ProfileTest));
    assert_eq!(keymap.map(key(KeyCode::F(5)), &app), None);
    assert_eq!(
        keymap.map(ctrl('s'), &app),
        Some(Action::ProfileSave { connect: false })
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), &app,),
        Some(Action::ProfileSave { connect: true })
    );
    assert_eq!(
        keymap.map(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT,),
            &app,
        ),
        Some(Action::ProfileSave { connect: true })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CloseProfileManager)
    );

    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Name;
    let text_mappings = [
        (KeyCode::Char('据'), Action::ProfileInsert('据'.into())),
        (KeyCode::Backspace, Action::ProfileBackspace),
        (KeyCode::Delete, Action::ProfileDeleteCharacter),
        (KeyCode::Left, Action::ProfileMoveLeft),
        (KeyCode::Right, Action::ProfileMoveRight),
        (KeyCode::Home, Action::ProfileMoveHome),
        (KeyCode::End, Action::ProfileMoveEnd),
    ];
    for (code, expected) in text_mappings {
        assert_eq!(keymap.map(key(code), &app), Some(expected));
    }
    for (event, expected) in [
        (ctrl('w'), Action::ProfileDeletePreviousWord),
        (ctrl('u'), Action::ProfileDeleteToStart),
        (ctrl('a'), Action::ProfileMoveHome),
        (ctrl('e'), Action::ProfileMoveEnd),
    ] {
        assert_eq!(keymap.map(event, &app), Some(expected));
    }
    for character in ['h', 'j', 'k', 'l'] {
        assert_eq!(
            keymap.map(key(KeyCode::Char(character)), &app),
            Some(Action::ProfileInsert(character.into()))
        );
    }
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::ProfileFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::ProfileFieldNext)
    );
    assert_eq!(
        keymap.map(
            KeyEvent::new(
                KeyCode::Char('@'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
            &app,
        ),
        Some(Action::ProfileInsert('@'.into()))
    );

    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Url;
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ProfileCommitUrl)
    );

    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Kind;
    assert_eq!(
        keymap.map(key(KeyCode::Left), &app),
        Some(Action::ProfileCycle(-1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Right), &app),
        Some(Action::ProfileCycle(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::ProfileCycle(-1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::ProfileCycle(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Up), &app),
        Some(Action::ProfileFieldPrevious)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::ProfileFieldNext)
    );
    assert_eq!(keymap.map(key(KeyCode::Enter), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);

    for field in [ProfileField::SslMode, ProfileField::Environment] {
        app.profile_manager.as_mut().unwrap().selected_field = field;
        for (code, expected) in [
            (KeyCode::Left, Action::ProfileCycle(-1)),
            (KeyCode::Char('h'), Action::ProfileCycle(-1)),
            (KeyCode::Right, Action::ProfileCycle(1)),
            (KeyCode::Char('l'), Action::ProfileCycle(1)),
            (KeyCode::Enter, Action::ProfileCycle(1)),
            (KeyCode::Char(' '), Action::ProfileCycle(1)),
            (KeyCode::Up, Action::ProfileFieldPrevious),
            (KeyCode::Char('k'), Action::ProfileFieldPrevious),
            (KeyCode::Down, Action::ProfileFieldNext),
            (KeyCode::Char('j'), Action::ProfileFieldNext),
        ] {
            assert_eq!(keymap.map(key(code), &app), Some(expected));
        }
    }

    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::ReadOnly;
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::ProfileToggle)
    );

    for (field, expected) in [
        (ProfileField::Test, Action::ProfileTest),
        (ProfileField::Save, Action::ProfileSave { connect: false }),
        (
            ProfileField::SaveAndConnect,
            Action::ProfileSave { connect: true },
        ),
        (ProfileField::Cancel, Action::CloseProfileManager),
    ] {
        app.profile_manager.as_mut().unwrap().selected_field = field;
        assert_eq!(keymap.map(key(KeyCode::Enter), &app), Some(expected));
    }
}

#[test]
fn profile_confirmation_and_paste_are_contextual_and_redacted() {
    let profile = profile("delete");
    let mut app = App::new(vec![profile]);
    let profile_id = app.profiles[0].id;
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileRequestDelete {
        profile_id: app.profiles[0].id,
    });
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ProfileConfirmDelete)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), &app,),
        None
    );
    assert_eq!(keymap.map(ctrl('y'), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::ProfileCancelDelete)
    );

    app.update(Action::ProfileCancelDelete);
    app.update(Action::ProfileStartEdit {
        profile_id: app.profiles[0].id,
    });
    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Password;
    let actions = map_paste("do-not-print".into(), &app);
    assert_eq!(actions, [Action::ProfilePaste("do-not-print".into())]);
    assert!(!format!("{actions:?}").contains("do-not-print"));

    app.update(Action::CloseProfileManager);
    app.update(Action::CloseProfileManager);
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Char('i'))));
    app.overlay = Some(Overlay::Help(lazydb::help::HelpState::new(
        lazydb::help::ShortcutContext::EditorNormal,
        lazydb::help::ShortcutCapabilities::default(),
    )));
    assert_eq!(
        map_paste("hidden".into(), &app),
        [Action::HelpPaste("hidden".into())]
    );
    app.overlay = Some(Overlay::Message {
        title: "Notice".into(),
        body: "Body".into(),
    });
    assert!(map_paste("hidden".into(), &app).is_empty());
    app.overlay = None;
    assert_eq!(
        map_paste("a\nb".into(), &app),
        [Action::EditorPaste("a\nb".into()),]
    );
}
