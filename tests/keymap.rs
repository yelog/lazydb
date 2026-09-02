use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
        Some(Action::HelpInsert('q'))
    );
    app.update(Action::HelpInsert('q'));
    assert_eq!(
        keymap.map(key(KeyCode::Backspace), &app),
        Some(Action::HelpBackspace)
    );
    app.update(Action::HelpBackspace);
    assert_eq!(keymap.map(ctrl('u'), &app), Some(Action::HelpClear));
    app.update(Action::HelpClear);
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
fn explorer_catalog_shortcuts_are_not_advertised_for_synthetic_rows() {
    let profile = profile("sqlite");
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::ProfileGroupCreate)
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
                selected_field: 0,
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
                selected_field: 0,
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
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::NewConsole)
    );
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
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
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

    assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('T')), &app),
        Some(Action::PreviousTab)
    );
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
fn space_n_opens_console_from_explorer_on_relation_tab() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::NewConsole)
    );
}

#[test]
fn space_s_returns_to_the_first_sql_console_from_explorer() {
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
        Some(Action::GotoSqlConsole)
    );
    app.update(Action::GotoSqlConsole);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn space_s_takes_priority_over_sql_result_order_input() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::GotoSqlConsole)
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

#[test]
fn materialized_view_create_form_maps_space_to_data_state_toggle() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
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
                    selected_field: 5,
                    query_editable: true,
                },
            ),
        ),
        baseline: None,
        plan: None,
        error: None,
    });
    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.overlay = Some(Overlay::CatalogEditor);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::CatalogEditorToggleMaterializedViewData)
    );
    app.update(Action::CatalogEditorToggleMaterializedViewData);
    let Some(lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("materialized view draft expected");
    };
    assert!(!draft.with_data);
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

    assert_eq!(keymap.map(ctrl('n'), &app), None);
    assert_eq!(keymap.map(ctrl('p'), &app), None);
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
        },
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
        Some(Action::ProfileGroupCreate)
    );
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
        Some(Action::HelpInsert('?'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::HelpInsert('q'))
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
        },
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
