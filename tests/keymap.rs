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
        notification::NotificationLevel,
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
fn mongo_console_transaction_shortcuts_are_contextual() {
    let mut app = App::new(vec![
        import_connection_url("mongodb://localhost:27017/app", Some("mongo"))
            .unwrap()
            .profile,
    ]);
    let tab = lazydb::model::mongodb_console::MongoConsoleTab::new(app.profiles[0].id, "app");
    app.tabs.push(WorkspaceTab::MongoConsole(tab));
    app.active_tab = 0;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT), &app),
        Some(Action::MongoTransactionBegin)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT), &app),
        Some(Action::MongoTransactionCommit)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT), &app),
        Some(Action::MongoTransactionAbort)
    );
}

#[test]
fn mongo_principal_overlay_edits_secret_and_requires_exact_confirmation() {
    let profile = import_connection_url("mongodb://localhost:27017/app", Some("mongo"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(&app.profiles[0]);
    app.sessions.register_attempt(
        target,
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation: 8,
        },
    );
    app.tabs.push(WorkspaceTab::MongoDashboard(Box::new(
        lazydb::model::dashboard::MongoDashboardTab {
            connection_generation: Some(8),
            ..lazydb::model::dashboard::MongoDashboardTab::new(profile_id, "app")
        },
    )));
    app.active_tab = 0;
    app.update(Action::MongoPrincipalNewUser);
    let mut keymap = Keymap::default();
    assert!(matches!(
        app.overlay,
        Some(Overlay::MongoPrincipalMutation(_))
    ));
    app.update(Action::MongoPrincipalMutationMoveField(1));
    for ch in "reader".chars() {
        app.update(Action::MongoPrincipalMutationEdit(TextInputEdit::Insert(
            ch,
        )));
    }
    app.update(Action::MongoPrincipalMutationMoveField(1));
    for ch in "top-secret".chars() {
        app.update(Action::MongoPrincipalMutationPasswordInsert(ch));
    }
    if let Some(action) = keymap.map(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        &app,
    ) {
        app.update(action);
    }
    let Some(Overlay::MongoPrincipalMutation(draft)) = app.overlay.as_ref() else {
        panic!("draft should remain open")
    };
    assert_eq!(draft.password_value(), "top-secret");
    assert!(!format!("{draft:?}").contains("top-secret"));
    app.update(Action::MongoPrincipalMutationConfirm);
    assert!(matches!(
        app.overlay,
        Some(Overlay::MongoPrincipalConfirmation { .. })
    ));
    for ch in "app:reader".chars() {
        app.update(Action::MongoPrincipalConfirmationInsert(ch));
    }
    let commands = app.update(Action::MongoPrincipalConfirmationConfirm);
    assert!(matches!(
        commands.as_slice(),
        [lazydb::action::Command::ExecuteMongoPrincipalMutation { .. }]
    ));
    assert_eq!(app.overlay, None);
}

#[test]
fn mongo_principal_operation_requires_exact_target_and_open_generation() {
    let profile = import_connection_url("mongodb://localhost:27017/app", Some("mongo"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
    let identity = lazydb::identity::ConnectionIdentity {
        profile_id,
        generation: 9,
    };
    app.sessions.register_attempt(target, identity);
    app.tabs.push(WorkspaceTab::MongoDashboard(Box::new(
        lazydb::model::dashboard::MongoDashboardTab {
            connection_generation: Some(9),
            ..lazydb::model::dashboard::MongoDashboardTab::new(profile_id, "app")
        },
    )));
    app.active_tab = 0;
    app.update(Action::MongoPrincipalNewUser);
    app.update(Action::MongoPrincipalMutationMoveField(1));
    for ch in "reader".chars() {
        app.update(Action::MongoPrincipalMutationEdit(TextInputEdit::Insert(
            ch,
        )));
    }
    app.update(Action::MongoPrincipalMutationMoveField(1));
    for ch in "secret".chars() {
        app.update(Action::MongoPrincipalMutationPasswordInsert(ch));
    }
    app.update(Action::MongoPrincipalMutationConfirm);
    for ch in "app:reader".chars() {
        app.update(Action::MongoPrincipalConfirmationInsert(ch));
    }
    let commands = app.update(Action::MongoPrincipalConfirmationConfirm);
    assert!(
        matches!(commands.as_slice(), [lazydb::action::Command::ExecuteMongoPrincipalMutation { profile_id: id, connection_generation: 9, .. }] if *id == profile_id)
    );
}

#[test]
fn stale_mongo_principal_failure_does_not_clear_another_pending_target() {
    let profile = import_connection_url("mongodb://localhost:27017/app", Some("mongo"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
    let identity = lazydb::identity::ConnectionIdentity {
        profile_id,
        generation: 9,
    };
    app.sessions.register_attempt(target, identity);
    app.tabs.push(WorkspaceTab::MongoDashboard(Box::new(
        lazydb::model::dashboard::MongoDashboardTab {
            connection_generation: Some(9),
            principal_pending_target: Some("app:reader".into()),
            principal_pending_operation: Some("dropUser".into()),
            principal_pending_connection_generation: Some(9),
            ..lazydb::model::dashboard::MongoDashboardTab::new(profile_id, "app")
        },
    )));

    app.update(Action::MongoPrincipalMutationFailed {
        profile_id,
        connection_generation: 9,
        database: "other_db".into(),
        command_name: "dropUser".into(),
        target: "reader".into(),
        confirmation_target: "other_db:reader".into(),
        message: "stale failure".into(),
    });

    let Some(WorkspaceTab::MongoDashboard(tab)) = app.tabs.first() else {
        panic!("Mongo dashboard should remain open")
    };
    assert_eq!(tab.principal_pending_target.as_deref(), Some("app:reader"));
    assert_eq!(tab.principal_pending_operation.as_deref(), Some("dropUser"));
}

fn control(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn press_leader_menu(keymap: &mut Keymap, app: &mut App, event: KeyEvent) {
    if let Some(action) = keymap.map(event, app) {
        app.update(action);
    }
}

#[test]
fn leader_menu_editor_cancel_clears_the_hidden_binding() {
    for cancel in [key(KeyCode::Esc), control(KeyCode::Char('c'))] {
        for nested in [false, true] {
            let mut app = App::new(Vec::new());
            app.update(Action::EditorKey(key(KeyCode::Esc)));
            let mut keymap = Keymap::default();
            press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(' ')));
            if nested {
                press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('t')));
            }
            press_leader_menu(&mut keymap, &mut app, cancel);
            assert!(
                keymap
                    .sequence_state(&app, std::time::Instant::now())
                    .is_none()
            );
            press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
            assert_eq!(app.overlay, None, "cancel={cancel:?}, nested={nested}");
        }
    }
}

#[test]
fn leader_menu_editor_repeated_prefix_keeps_the_hint_and_binding() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();
    press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(' ')));
    let mut repeat = key(KeyCode::Char(' '));
    repeat.kind = KeyEventKind::Repeat;
    press_leader_menu(&mut keymap, &mut app, repeat);
    assert!(
        keymap
            .sequence_state(&app, std::time::Instant::now())
            .is_some()
    );
    press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
    assert!(matches!(app.overlay, Some(Overlay::SqlEditorList(_))));
}

#[test]
fn leader_menu_editor_other_keys_do_not_leave_a_hidden_binding() {
    for event in [
        key(KeyCode::Left),
        key(KeyCode::F(3)),
        control(KeyCode::Char('r')),
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::SUPER),
    ] {
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        let mut keymap = Keymap::default();
        press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(' ')));
        press_leader_menu(&mut keymap, &mut app, event);
        assert!(
            keymap
                .sequence_state(&app, std::time::Instant::now())
                .is_none()
        );
        press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
        assert_eq!(app.overlay, None, "event={event:?}");
    }
}

#[test]
fn leader_menu_editor_alias_and_visual_cancel_preserve_the_mode() {
    for visual in [false, true] {
        for prefix in [' ', '\\'] {
            let mut app = App::new(Vec::new());
            app.update(Action::EditorKey(key(KeyCode::Esc)));
            if visual {
                app.update(Action::EditorKey(key(KeyCode::Char('v'))));
            }
            let mode = app.active_editor_mode();
            let mut keymap = Keymap::default();
            press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(prefix)));
            let later = std::time::Instant::now() + std::time::Duration::from_secs(60);
            assert!(keymap.sequence_state(&app, later).is_some());
            press_leader_menu(&mut keymap, &mut app, key(KeyCode::Esc));
            assert!(keymap.sequence_state(&app, later).is_none());
            assert_eq!(app.active_editor_mode(), mode);
            if visual {
                app.update(Action::EditorKey(key(KeyCode::Esc)));
            }
            press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
            assert_eq!(app.overlay, None);
        }
    }
}

#[test]
fn leader_menu_enter_clears_the_editor_binding_after_execution() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();
    press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(' ')));
    press_leader_menu(&mut keymap, &mut app, key(KeyCode::Enter));
    assert!(
        keymap
            .sequence_state(&app, std::time::Instant::now())
            .is_none()
    );
    assert_eq!(app.overlay, None);
    press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
    assert_eq!(app.overlay, None);
}

#[test]
fn leader_menu_editor_context_change_does_not_restore_the_hidden_binding() {
    for change in ["focus", "tab", "overlay", "omni"] {
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        let mut keymap = Keymap::default();
        press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char(' ')));
        let original = app.active_tab;
        match change {
            "focus" => {
                app.update(Action::Focus(Focus::Results));
                assert!(
                    keymap
                        .sequence_state(&app, std::time::Instant::now())
                        .is_none()
                );
                app.update(Action::Focus(Focus::Editor));
            }
            "tab" => {
                app.update(Action::NewConsole);
                assert!(
                    keymap
                        .sequence_state(&app, std::time::Instant::now())
                        .is_none()
                );
                app.update(Action::ActivateTab(original));
            }
            "overlay" => {
                app.update(Action::ShowHelp);
                assert!(
                    keymap
                        .sequence_state(&app, std::time::Instant::now())
                        .is_none()
                );
                app.update(Action::DismissOverlay);
            }
            "omni" => {
                app.update(Action::OpenOmni);
                assert!(
                    keymap
                        .sequence_state(&app, std::time::Instant::now())
                        .is_none()
                );
                app.update(Action::OmniDismiss);
            }
            _ => unreachable!(),
        }
        press_leader_menu(&mut keymap, &mut app, key(KeyCode::Char('s')));
        assert_eq!(app.overlay, None, "change={change}");
    }
}

#[test]
fn pane_binding_cmd_ctrl_h_moves_focus_from_sql_modes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.toml");
    std::fs::write(
        &settings,
        "[keybindings.panes]\nfocus-pane-left = [\"Cmd+Ctrl+h\"]\n",
    )
    .unwrap();
    let config = lazydb::config::AppConfig::load(settings).unwrap();
    let bindings = config.keybindings.key_bindings().unwrap();
    let timeout = std::time::Duration::from_millis(config.keybindings.sequence_timeout_ms);
    let event = KeyEvent::new(
        KeyCode::Char('h'),
        KeyModifiers::SUPER | KeyModifiers::CONTROL,
    );
    for (focus, mode) in [
        (Focus::Results, None),
        (Focus::Editor, Some(EditorMode::Normal)),
        (Focus::Editor, Some(EditorMode::Insert)),
    ] {
        let mut keymap = Keymap::with_sequence_timeout_and_bindings(timeout, bindings.clone());
        let mut app = App::new(Vec::new());
        if let Some(mode) = mode {
            app.update(Action::EditorKey(key(KeyCode::Esc)));
            if mode == EditorMode::Insert {
                app.update(Action::EditorKey(key(KeyCode::Char('i'))));
            }
        }
        app.focus = focus;

        let action = keymap.map(event, &app).expect("configured pane action");
        app.update(action);

        assert_eq!(app.focus, Focus::Explorer, "mode={mode:?}");
        if let Some(mode) = mode {
            assert_eq!(app.active_editor_mode(), mode);
        }
    }
}

#[test]
fn pane_binding_rejects_ctrl_w_as_a_prefix_when_ctrl_w_cycles_panes() {
    let mut config = lazydb::config::AppConfig::default();
    config.keybindings.panes.insert(
        "focus-pane-left".into(),
        vec!["Ctrl-w h".into(), "Ctrl-w Ctrl-w".into()],
    );
    let result = config.keybindings.key_bindings();
    assert!(result.is_err());
}

#[test]
fn pane_binding_accepts_custom_two_key_prefix() {
    let mut config = lazydb::config::AppConfig::default();
    config
        .keybindings
        .panes
        .insert("focus-pane-right".into(), vec!["Alt+x h".into()]);
    let bindings = config.keybindings.key_bindings().unwrap();
    let mut keymap =
        Keymap::with_sequence_timeout_and_bindings(std::time::Duration::from_millis(750), bindings);
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT), &app),
        None
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::Focus(Focus::Editor))
    );
    app.update(Action::Focus(Focus::Editor));
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn pane_binding_accepts_three_key_sequence_and_empty_array_disables() {
    let mut config = lazydb::config::AppConfig::default();
    config
        .keybindings
        .panes
        .insert("focus-pane-right".into(), vec!["Alt+x a h".into()]);
    config
        .keybindings
        .panes
        .insert("focus-pane-left".into(), Vec::new());
    let bindings = config.keybindings.key_bindings().unwrap();
    assert!(!bindings.configured_for("focus-pane-left"));
    assert_eq!(bindings.configured_sequences("focus-pane-right").len(), 1);

    let mut keymap =
        Keymap::with_sequence_timeout_and_bindings(std::time::Duration::from_millis(750), bindings);
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    for event in [
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
        key(KeyCode::Char('a')),
    ] {
        assert_eq!(keymap.map(event, &app), None);
    }
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::Focus(Focus::Editor))
    );
}

#[test]
fn sql_history_overlay_routes_navigation_by_mode() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    app.overlay = Some(Overlay::SqlHistory(Default::default()));
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('/')), &app),
        Some(Action::SqlHistorySearchOpen)
    );
    app.update(Action::SqlHistorySearchOpen);
    assert_eq!(
        keymap.map(key(KeyCode::Backspace), &app),
        Some(Action::SqlHistorySearchBackspace)
    );
    app.update(Action::SqlHistorySearchConfirm);
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SqlHistoryOpenDetail)
    );
    app.update(Action::SqlHistoryOpenDetail);
    assert!(matches!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ReadOnlyEditorKey { .. })
    ));
}

#[test]
fn redis_keys_routes_before_generic_results_navigation_and_supports_find() {
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(
            Uuid::from_u128(10),
            lazydb::db::redis::types::RedisTarget {
                profile_id: Uuid::from_u128(11),
                database: 0,
            },
        ),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    let action = keymap.map(key(KeyCode::Char('/')), &app).unwrap();
    assert_eq!(action, Action::RedisFindOpen);
    app.update(action);
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::RedisFindInsert('j'))
    );
    app.update(Action::RedisFindConfirm);
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::RedisMoveSelection(1))
    );
    let action = keymap.map(key(KeyCode::Enter), &app).unwrap();
    assert_eq!(action, Action::RedisPrimarySelection);
}

#[test]
fn redis_keys_o_toggles_selected_group_before_results_bindings() {
    use lazydb::db::redis::types::{RedisKeyId, RedisTarget};
    use lazydb::model::redis_browser::RedisBrowserTab;
    use lazydb::model::redis_key_tree::KeyTreeNodeId;

    let mut app = App::new(Vec::new());
    let target = RedisTarget {
        profile_id: Uuid::from_u128(101),
        database: 0,
    };
    let mut tab = RedisBrowserTab::new(Uuid::from_u128(102), target.clone());
    let prefix = KeyTreeNodeId::Prefix(b"users:".to_vec());
    tab.tree.rebuild(&[
        RedisKeyId {
            target: target.clone(),
            key: b"users:1".to_vec(),
        },
        RedisKeyId {
            target,
            key: b"users:2".to_vec(),
        },
    ]);
    tab.select(Some(prefix.clone()));
    let tab_id = tab.id;
    app.tabs.push(WorkspaceTab::RedisBrowser(tab));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    for expanded in [true, false] {
        let action = keymap.map(key(KeyCode::Char('o')), &app).unwrap();
        assert_eq!(
            action,
            Action::RedisToggleNode {
                tab_id,
                node: prefix.clone()
            }
        );
        assert!(app.update(action).is_empty());
        let WorkspaceTab::RedisBrowser(tab) = &app.tabs[app.active_tab] else {
            panic!("expected Redis browser tab");
        };
        assert_eq!(tab.tree.expanded.contains(&prefix), expanded);
        assert_eq!(tab.tree.selected.as_ref(), Some(&prefix));
        assert!(tab.opened_key.is_none());
        assert_eq!(tab.preview_generation, 0);
    }
}

#[test]
fn redis_keys_use_h_and_l_for_tree_depth_navigation() {
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(
            Uuid::from_u128(12),
            lazydb::db::redis::types::RedisTarget {
                profile_id: Uuid::from_u128(13),
                database: 0,
            },
        ),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::RedisCollapseSelection)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::RedisExpandSelection)
    );
}

#[test]
fn mongodb_document_editor_only_saves_on_control_enter_and_accepts_text() {
    let mut app = App::new(Vec::new());
    let mut tab = lazydb::model::mongodb_browser::MongoBrowserTab::new(
        Uuid::from_u128(900),
        lazydb::db::mongodb::types::MongoNamespace::collection("app", "items"),
    );
    let bson = mongodb::bson::doc! { "_id": 1_i32 };
    tab.page = lazydb::model::mongodb_browser::MongoPageState::Ready(
        lazydb::db::mongodb::types::MongoDocumentPage {
            documents: vec![lazydb::db::mongodb::types::MongoDocument {
                raw: mongodb::bson::raw::RawDocumentBuf::from_bytes(
                    mongodb::bson::to_vec(&bson).unwrap(),
                )
                .unwrap(),
                bson,
                editable: true,
            }],
            next_offset: None,
            truncated: false,
            oversized_document: false,
            bytes: 16,
        },
    );
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::MongoBrowser(tab));
    app.active_tab = 0;
    app.focus = Focus::Results;
    assert!(app.update(Action::MongoDocumentEdit).is_empty());
    let mut keymap = Keymap::default();
    assert!(matches!(app.tabs[0], WorkspaceTab::MongoBrowser(ref tab) if tab.editing));
    assert_eq!(app.focus, Focus::Results);

    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::MongoDocumentEditText(TextInputEdit::Insert('x')))
    );
    assert_eq!(keymap.map(key(KeyCode::Enter), &app), None);
    assert_eq!(
        keymap.map(control(KeyCode::Enter), &app),
        Some(Action::MongoDocumentSave)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::MongoDocumentCancelEdit)
    );
}

#[test]
fn mongo_management_drafts_open_for_create_view_index_and_rename() {
    use lazydb::db::mongodb::types::{
        MongoCollectionInfo, MongoCollectionType, MongoExplorerNodeId, MongoNamespace,
    };
    use lazydb::model::explorer::{ProfilePlacement, ProfileProvenance};

    let namespace = MongoNamespace::collection("app", "items");
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.profiles.clear();
    let imported =
        lazydb::profile::import_connection_url("mongodb://localhost/app", Some("mongo")).unwrap();
    let profile = imported.profile;
    let profile_id = profile.id;
    app.profiles.push(profile);
    app.explorer.normalized.profiles.insert(
        profile_id,
        lazydb::model::explorer::ExplorerProfileState::new(
            profile_id,
            "mongo".into(),
            lazydb::profile::DatabaseKind::MongoDb,
            "localhost".into(),
            ProfileProvenance::Saved,
            ProfilePlacement::Global,
        ),
    );
    let mut keymap = Keymap::default();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Database("app".into()),
    });
    assert!(matches!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::MongoManagementOpen(
            lazydb::model::mongodb_management::MongoManagementOperation::CreateCollection { .. }
        ))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::MongoManagementOpen(
            lazydb::model::mongodb_management::MongoManagementOperation::CreateView { .. }
        ))
    ));
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .mongodb_collections
        .insert(
            "app".into(),
            vec![MongoCollectionInfo {
                namespace: namespace.clone(),
                collection_type: MongoCollectionType::Collection,
                options_ejson: None,
            }],
        );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Collection(namespace.clone()),
    });
    assert!(matches!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::MongoManagementOpen(
            lazydb::model::mongodb_management::MongoManagementOperation::CreateIndex { .. }
        ))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('R')), &app),
        Some(Action::MongoManagementOpen(
            lazydb::model::mongodb_management::MongoManagementOperation::RenameCollection { .. }
        ))
    ));
    assert!(matches!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::MongoManagementOpen(
            lazydb::model::mongodb_management::MongoManagementOperation::DropCollection { .. }
        ))
    ));
}

#[test]
fn redis_preview_routes_vim_motion_to_its_read_only_editor() {
    let target = lazydb::db::redis::types::RedisTarget {
        profile_id: Uuid::from_u128(30),
        database: 0,
    };
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(Uuid::from_u128(31), target),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[app.active_tab] {
        tab.focus = lazydb::model::redis_browser::RedisBrowserFocus::Preview;
    }

    let mut keymap = Keymap::default();
    for code in [
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
    ] {
        assert!(
            matches!(
                keymap.map(key(code), &app),
                Some(Action::ReadOnlyEditorKey { session_id, event })
                    if event.code == code
                        && session_id
                            == match &app.tabs[app.active_tab] {
                                WorkspaceTab::RedisBrowser(tab) => tab.preview_editor_id,
                                _ => unreachable!(),
                            }
            ),
            "{code:?} should be routed to the Preview editor"
        );
    }
}

#[test]
fn redis_table_preview_routes_motion_to_the_shared_grid() {
    let target = lazydb::db::redis::types::RedisTarget {
        profile_id: Uuid::from_u128(35),
        database: 0,
    };
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(Uuid::from_u128(36), target),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[app.active_tab] {
        tab.focus = lazydb::model::redis_browser::RedisBrowserFocus::Preview;
        tab.format
            .select(lazydb::value_preview::PreviewFormat::TABLE);
    }

    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::GridMove {
            rows: 1,
            columns: 0,
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::GridMove {
            rows: 0,
            columns: -1,
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char(']')), &app),
        Some(Action::GridResizeColumn(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::CopyGridCell)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('/')), &app),
        Some(Action::RedisValueFilterFocus {
            tab_id: Uuid::from_u128(36)
        })
    );
    assert_eq!(keymap.map(key(KeyCode::Char('W')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::RedisPreviewEdit)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::RedisPreviewAdd)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('d')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::RedisPreviewDelete)
    );
}

#[test]
fn redis_preview_keeps_application_controls_outside_the_editor_stream() {
    let target = lazydb::db::redis::types::RedisTarget {
        profile_id: Uuid::from_u128(32),
        database: 0,
    };
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(Uuid::from_u128(33), target),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[app.active_tab] {
        tab.focus = lazydb::model::redis_browser::RedisBrowserFocus::Preview;
    }

    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert!(matches!(
        keymap.map(key(KeyCode::Char('f')), &app),
        Some(Action::RedisPreviewCycleFormat)
    ));
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('w')), &app),
        Some(Action::RedisPreviewToggleWrap)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::RedisPreviewLoadNext)
    );
    assert!(matches!(
        keymap.map(key(KeyCode::PageDown), &app),
        Some(Action::ReadOnlyEditorKey { .. })
    ));
}

#[test]
fn redis_save_confirm_routes_save_and_cancel_without_leaking_to_editor() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    app.overlay = Some(Overlay::RedisValueSaveConfirm {
        tab_id: Uuid::from_u128(40),
        revision: 2,
        invalid: false,
        validation_error: None,
        focus: 0,
        format: lazydb::value_preview::PreviewFormat::RAW,
    });
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RedisValueSaveActivate(0))
    );
    app.overlay = Some(Overlay::RedisValueSaveConfirm {
        tab_id: Uuid::from_u128(40),
        revision: 2,
        invalid: true,
        validation_error: Some("invalid JSON".into()),
        focus: 0,
        format: lazydb::value_preview::PreviewFormat::JSON,
    });
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RedisValueSaveActivate(0))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RedisValueSaveCancel)
    );
}

#[test]
fn redis_keys_keep_tree_navigation_when_preview_routing_is_enabled() {
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(
            Uuid::from_u128(34),
            lazydb::db::redis::types::RedisTarget {
                profile_id: Uuid::from_u128(35),
                database: 0,
            },
        ),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::RedisMoveSelection(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::RedisCollapseSelection)
    );
}

#[test]
fn redis_keys_route_y_and_d_before_generic_results_bindings() {
    let mut app = App::new(Vec::new());
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(
            Uuid::from_u128(14),
            lazydb::db::redis::types::RedisTarget {
                profile_id: Uuid::from_u128(15),
                database: 0,
            },
        ),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::RedisCopyKey)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::RedisDeleteKey)
    );
}

#[test]
fn redis_object_actions_open_from_prefix_and_key_and_edit_in_the_overlay() {
    let target = lazydb::db::redis::types::RedisTarget {
        profile_id: Uuid::from_u128(21),
        database: 0,
    };
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.normalized.selected =
        Some(lazydb::model::explorer::ExplorerNodeId::RedisDatabase {
            profile_id: target.profile_id,
            database: target.database,
        });
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenRedisObjectCreateAt {
            profile_id: target.profile_id,
            database: target.database,
        })
    );
    app.explorer.normalized.selected = None;
    app.tabs.push(WorkspaceTab::RedisBrowser(
        lazydb::model::redis_browser::RedisBrowserTab::new(Uuid::from_u128(20), target.clone()),
    ));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[app.active_tab] {
        tab.tree.selected = Some(lazydb::model::redis_key_tree::KeyTreeNodeId::Prefix(
            b"cache:".to_vec(),
        ));
    }
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenRedisObjectCreate)
    );
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[app.active_tab] {
        tab.tree.selected = Some(lazydb::model::redis_key_tree::KeyTreeNodeId::Key(
            b"cache:key".to_vec(),
        ));
    }
    assert_eq!(
        keymap.map(key(KeyCode::Char('e')), &app),
        Some(Action::OpenRedisObjectEdit)
    );

    let editor = lazydb::model::redis_object_editor::RedisObjectEditorState::create(
        Uuid::from_u128(22),
        lazydb::identity::ConnectionIdentity {
            profile_id: target.profile_id,
            generation: 1,
        },
        target,
    );
    app.overlay = Some(Overlay::RedisObjectEditor(Box::new(editor)));
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::RedisObjectEditorFocusNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::RedisObjectEditorInsert('x'))
    );
}

#[test]
fn profile_group_delete_navigation_and_enter_follow_selected_button() {
    use lazydb::model::profile_group::ProfileGroupOverlay;

    for event in [
        key(KeyCode::Left),
        key(KeyCode::Right),
        key(KeyCode::Tab),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    ] {
        let group_id = Uuid::from_u128(99);
        let mut app = App::new(Vec::new());
        app.explorer.normalized.selected = Some(ExplorerNodeId::ConnectionGroup {
            group_id,
            region: lazydb::model::explorer::ProfileRegion::Primary,
        });
        let mut keymap = Keymap::default();
        for cancel in [true, false] {
            assert!(app.update(Action::ProfileGroupDeleteConfirm).is_empty());
            assert_eq!(
                keymap.map(key(KeyCode::Enter), &app),
                Some(Action::ProfileGroupConfirm)
            );
            let action = keymap.map(event, &app).unwrap();
            assert_eq!(action, Action::ToggleProfileGroupDeleteFocus);
            assert!(app.update(action).is_empty());
            assert!(matches!(
                app.overlay,
                Some(Overlay::ProfileGroup(ProfileGroupOverlay::DeleteConfirm {
                    cancel_selected: true,
                    ..
                }))
            ));
            assert_eq!(
                keymap.map(key(KeyCode::Char('y')), &app),
                Some(Action::ProfileGroupConfirm)
            );
            for code in [KeyCode::Esc, KeyCode::Char('n'), KeyCode::Char('q')] {
                assert_eq!(
                    keymap.map(key(code), &app),
                    Some(Action::ProfileGroupCancel)
                );
            }
            if !cancel {
                let action = keymap.map(event, &app).unwrap();
                assert!(app.update(action).is_empty());
            }
            let action = keymap.map(key(KeyCode::Enter), &app).unwrap();
            assert_eq!(
                action,
                if cancel {
                    Action::ProfileGroupCancel
                } else {
                    Action::ProfileGroupConfirm
                }
            );
            let commands = app.update(action);
            if cancel {
                assert!(commands.is_empty());
            } else {
                assert!(
                    matches!(commands.as_slice(), [lazydb::action::Command::UpdateProfileOrganization {
                    mutation: lazydb::action::ProfileOrganizationMutation::DeleteGroup { group_id: found }, ..
                }] if *found == group_id)
                );
            }
            assert!(app.overlay.is_none());
        }
    }
}

#[test]
fn result_pagination_keys_map_in_sql_and_relation_data_contexts() {
    let mut sql = App::new(Vec::new());
    sql.focus = Focus::Results;
    let mut keymap = Keymap::default();
    for (event, action) in [
        (key(KeyCode::PageUp), Action::ResultPreviousPage),
        (key(KeyCode::PageDown), Action::ResultNextPage),
        (control(KeyCode::Home), Action::ResultFirstPage),
        (control(KeyCode::End), Action::ResultLastPage),
    ] {
        assert_eq!(keymap.map(event, &sql), Some(action));
    }
    assert_eq!(
        keymap.map(key(KeyCode::Char('P')), &sql),
        Some(Action::OpenPageSizeSelector { relation: false })
    );

    let mut relation = App::new(Vec::new());
    relation
        .tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    relation.active_tab = relation.tabs.len() - 1;
    relation.focus = Focus::Results;
    let mut keymap = Keymap::default();
    for (event, action) in [
        (key(KeyCode::PageUp), Action::RelationPreviousPage),
        (key(KeyCode::PageDown), Action::RelationNextPage),
        (control(KeyCode::Home), Action::RelationFirstPage),
        (control(KeyCode::End), Action::RelationLastPage),
    ] {
        assert_eq!(keymap.map(event, &relation), Some(action));
    }
    assert_eq!(
        keymap.map(key(KeyCode::Char('P')), &relation),
        Some(Action::OpenPageSizeSelector { relation: true })
    );
}

#[test]
fn temporal_editor_owns_field_and_month_navigation() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        let mut edit = lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
            lazydb::db::value::CellValue::Date(
                chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
            ),
        ]]);
        edit.mode = lazydb::model::relation_edit::RelationGridMode::EditCell(Box::new(
            lazydb::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: lazydb::model::cell_editor::CellEditorBuffer {
                    presence: lazydb::model::cell_editor::CellEditorPresence::Value,
                    content: lazydb::model::cell_editor::CellEditorContent::Typed {
                        kind: lazydb::model::cell_editor::CellEditorKind::Date,
                        draft: lazydb::model::cell_editor::TypedDraft::Temporal(
                            lazydb::model::cell_editor::TemporalDraft::date(
                                chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                            ),
                        ),
                    },
                    ..lazydb::model::cell_editor::CellEditorBuffer::default()
                },
                error: None,
            },
        ));
        tab.edit = Some(edit);
    }
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Right), &app),
        Some(Action::RelationEditTemporalMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char(']')), &app),
        Some(Action::RelationEditTemporalMonth(1))
    );
}

#[test]
fn relation_cell_paste_routes_text_payload_instead_of_yanked_row() {
    use lazydb::model::relation_edit::{CellEditorState, RelationEditSession, RelationGridMode};

    let mut app = App::new(Vec::new());
    let mut relation = RelationTab::new("users");
    let mut edit = RelationEditSession::from_rows(vec![vec![lazydb::db::value::CellValue::Text(
        "original".into(),
    )]]);
    edit.yank = Some(vec![lazydb::db::value::CellValue::Text("yanked".into())]);
    edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
        row: 0,
        column: 0,
        input: lazydb::model::cell_editor::CellEditorBuffer::default(),
        error: None,
    }));
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    app.focus = Focus::Results;

    assert_eq!(
        map_paste("clipboard text".into(), &app),
        vec![Action::RelationEditPaste("clipboard text".into())]
    );
}

#[test]
fn paste_ignores_relation_query_focus_outside_results_and_busy_grid() {
    use lazydb::model::relation_edit::{RelationEditSession, RelationGridMode};

    let mut app = App::new(Vec::new());
    let mut relation = RelationTab::new("users");
    relation.query.capability = lazydb::model::data_query::DataQueryCapability::Relation;
    relation.query.focus = Some(DataQueryInput::Where);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;

    app.focus = Focus::Editor;
    assert_eq!(
        map_paste("editor paste".into(), &app),
        vec![Action::EditorPaste("editor paste".into())]
    );

    app.focus = Focus::Results;
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.query.focus = None;
        let mut edit = RelationEditSession::from_rows(Vec::new());
        edit.mode = RelationGridMode::Busy;
        tab.edit = Some(edit);
    }
    assert!(map_paste("ignored".into(), &app).is_empty());
}

#[test]
fn cell_presence_shortcuts_do_not_override_json_or_relation_transaction_keys() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        let mut edit = lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
            lazydb::db::value::CellValue::Null,
        ]]);
        edit.mode = lazydb::model::relation_edit::RelationGridMode::EditCell(Box::new(
            lazydb::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: lazydb::model::cell_editor::CellEditorBuffer {
                    presence: lazydb::model::cell_editor::CellEditorPresence::Null,
                    content: lazydb::model::cell_editor::CellEditorContent::Typed {
                        kind: lazydb::model::cell_editor::CellEditorKind::Json,
                        draft: lazydb::model::cell_editor::TypedDraft::Json(
                            lazydb::model::cell_editor::JsonBuffer::new("null"),
                        ),
                    },
                    presence_history: Vec::new(),
                    presence_redo: Vec::new(),
                },
                error: None,
            },
        ));
        tab.edit = Some(edit);
    }

    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT), &app),
        Some(Action::RelationEditSetNull)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT), &app),
        Some(Action::RelationEditRestoreDefault)
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT), &app),
        Some(Action::RelationEditUseValue)
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('s')), &app),
        Some(Action::RelationEditConfirm)
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('f')), &app),
        Some(Action::RelationEditJsonFormat)
    );
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
        keymap.map(text_undo(), &app),
        Some(Action::SqlEditorListInputUndo)
    );
    assert_eq!(
        keymap.map(text_redo(), &app),
        Some(Action::SqlEditorListInputRedo)
    );

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
fn transaction_menu_owns_navigation_and_cancel_keys() {
    let mut app = App::new(vec![profile("transaction-menu")]);
    app.overlay = Some(Overlay::TransactionMenu { selected: 0 });
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::MoveTransactionMenu(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ConfirmTransactionMenu)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CancelTransactionMenu)
    );
}

#[test]
fn transaction_menu_does_not_open_without_a_sql_console() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.update(Action::OpenTransactionMenu);
    assert_eq!(app.overlay, None);
}

#[test]
fn console_manager_delete_keys_are_confirmation_aware() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    app.update(Action::OpenSqlEditorList);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::SqlEditorListDeleteRequest)
    );
    app.update(Action::SqlEditorListDeleteRequest);
    assert_eq!(keymap.map(key(KeyCode::Char('y')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::SqlEditorListDeleteFocusNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SqlEditorListDeleteCancel)
    );
    app.update(Action::SqlEditorListDeleteFocusNext);
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::SqlEditorListDeleteConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::SqlEditorListDeleteCancel)
    );
}

#[test]
fn standalone_console_delete_enter_activates_the_focused_button() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    app.update(Action::RequestDeleteActiveConsole);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CancelDeleteConsole)
    );
    app.update(Action::ToggleDeleteConsoleFocus);
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ConfirmDeleteConsole)
    );
}

#[test]
fn catalog_discard_enter_activates_the_focused_button() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::CatalogEditorDiscardConfirm {
        focus: lazydb::model::workspace::CatalogEditorDiscardFocus::KeepEditing,
    });
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorDiscardKeepEditing)
    );
    app.update(Action::CatalogEditorDiscardMove(1));
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorDiscardChanges)
    );
}

#[test]
fn catalog_discard_navigation_supports_horizontal_and_tab_keys() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::CatalogEditorDiscardConfirm {
        focus: lazydb::model::workspace::CatalogEditorDiscardFocus::KeepEditing,
    });
    let mut keymap = Keymap::default();

    for event in [key(KeyCode::Right), key(KeyCode::Down), key(KeyCode::Tab)] {
        assert_eq!(
            keymap.map(event, &app),
            Some(Action::CatalogEditorDiscardMove(1))
        );
    }
    for event in [key(KeyCode::Left), key(KeyCode::Up), key(KeyCode::BackTab)] {
        assert_eq!(
            keymap.map(event, &app),
            Some(Action::CatalogEditorDiscardMove(-1))
        );
    }
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

fn text_history_modifier() -> KeyModifiers {
    if cfg!(target_os = "macos") {
        KeyModifiers::SUPER
    } else {
        KeyModifiers::CONTROL
    }
}

fn text_undo() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('z'), text_history_modifier())
}

fn text_redo() -> KeyEvent {
    KeyEvent::new(
        KeyCode::Char('z'),
        text_history_modifier() | KeyModifiers::SHIFT,
    )
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
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::CatalogEditorFieldNext)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::CatalogEditorFieldPrevious)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('d')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('d')), &app),
        Some(Action::CatalogEditorRemoveTableColumn)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
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
fn catalog_form_keymap_keeps_review_and_cancel_semantics() {
    let mut app = materialized_view_keymap_app();
    let mut keymap = Keymap::default();
    for (focus, expected) in [
        (
            lazydb::model::catalog_editor::CatalogFormFocus::Review,
            Action::CatalogEditorPreview,
        ),
        (
            lazydb::model::catalog_editor::CatalogFormFocus::Cancel,
            Action::CatalogEditorCancel,
        ),
    ] {
        match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
            lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft) => {
                draft.focus = focus
            }
            _ => panic!("materialized view draft expected"),
        }
        assert_eq!(keymap.map(key(KeyCode::Enter), &app), Some(expected));
    }
}

fn materialized_view_keymap_app() -> App {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        database_kind: None,
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: None,
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: Vec::new(),
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
                    focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
                    query_editable: true,
                    clickhouse_native: None,
                },
            ),
        ),
        baseline: None,
        plan: None,
        error: None,
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
    });
    app.overlay = Some(Overlay::CatalogEditor);
    app
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
        lazydb::model::catalog_editor::TableActionField::AddConstraintPrimaryKey,
        lazydb::model::catalog_editor::TableActionField::AddConstraintUnique,
        lazydb::model::catalog_editor::TableActionField::AddConstraintCheck,
        lazydb::model::catalog_editor::TableActionField::AddConstraintForeignKey,
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
            lazydb::model::catalog_editor::TableActionField::AddConstraintPrimaryKey => {
                Action::CatalogEditorAddTableConstraintPrimaryKey
            }
            lazydb::model::catalog_editor::TableActionField::AddConstraintUnique => {
                Action::CatalogEditorAddTableConstraintUnique
            }
            lazydb::model::catalog_editor::TableActionField::AddConstraintCheck => {
                Action::CatalogEditorAddTableConstraintCheck
            }
            lazydb::model::catalog_editor::TableActionField::AddConstraintForeignKey => {
                Action::CatalogEditorAddTableConstraintForeignKey
            }
            lazydb::model::catalog_editor::TableActionField::Review => Action::CatalogEditorPreview,
            lazydb::model::catalog_editor::TableActionField::Cancel => Action::CatalogEditorCancel,
            _ => unreachable!("only fixed action buttons are covered here"),
        };
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(expected.clone())
        );
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), Some(expected));
        if matches!(
            action_field,
            lazydb::model::catalog_editor::TableActionField::AddColumn
        ) {
            assert_eq!(
                keymap.map(key(KeyCode::Esc), &app),
                Some(Action::CatalogEditorCancel)
            );
        }
    }
}

#[test]
fn table_editor_opens_column_details_with_e_and_reviews_with_enter() {
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
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
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
fn table_editor_tab_visits_each_column_row() {
    let mut app = table_editor_app();
    let mut keymap = Keymap::default();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.columns = (0..100)
        .map(|index| {
            let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added();
            column.name = format!("column_{index}").into();
            column
        })
        .collect();
    draft.selected_column = 0;
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

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
        lazydb::model::catalog_editor::TableEditorFocus::Columns
    );
    assert_eq!(draft.selected_column, 1);
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
        database_kind: None,
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
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
        database_kind: None,
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
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
    app.update(Action::CatalogEditorFocusFormField(
        lazydb::model::catalog_editor::CatalogFormFocus::Review,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
    app.update(Action::CatalogEditorFocusFormField(
        lazydb::model::catalog_editor::CatalogFormFocus::Cancel,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorCancel)
    );

    app.update(Action::CatalogEditorFocusFormField(
        lazydb::model::catalog_editor::CatalogFormFocus::Password,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::CatalogEditorPasswordInsert('s'.into()))
    );
    let password_action = keymap.map(key(KeyCode::Char('s')), &app).unwrap();
    assert!(!format!("{password_action:?}").contains("ProfileInput(\"s\")"));
    assert!(!format!("{password_action:?}").contains("\"s\""));
    app.update(password_action);
    let role = match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .unwrap()
    {
        lazydb::model::catalog_editor::CatalogDraft::Role(role) => role,
        _ => panic!("role draft expected"),
    };
    assert_eq!(role.password.as_ref().unwrap().expose(), "s");
    assert!(!format!("{role:?}").contains("password: Some"));
    assert!(!format!("{role:?}").contains("SecretTextInput"));
}

#[test]
fn catalog_profile_form_keymap_inserts_text_into_all_three_create_forms() {
    let cases = [
        (
            lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Database,
            ),
            "database",
        ),
        (
            lazydb::db::catalog_mutation::CatalogObjectType::LoginRole,
            "login role",
        ),
        (
            lazydb::db::catalog_mutation::CatalogObjectType::Role,
            "role",
        ),
    ];

    for (object_type, label) in cases {
        let mut app = App::new(Vec::new());
        let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
            lazydb::db::catalog_mutation::CatalogMutationMode::Create,
            lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
                profile_id: Uuid::nil(),
            },
            0,
            vec![],
        );
        assert!(editor.select_object_type(object_type));
        app.catalog_editor = Some(editor);
        app.overlay = Some(Overlay::CatalogEditor);
        let mut keymap = Keymap::default();

        for character in ['a', '界', 'b'] {
            let action = keymap
                .map(key(KeyCode::Char(character)), &app)
                .unwrap_or_else(|| panic!("{label} should map printable text"));
            app.update(action);
        }

        let name = match app
            .catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
            .unwrap()
        {
            lazydb::model::catalog_editor::CatalogDraft::Database(draft) => draft.name.value(),
            lazydb::model::catalog_editor::CatalogDraft::Role(draft) => draft.name.value(),
            _ => panic!("{label} should create a database or role draft"),
        };
        assert_eq!(name, "a界b", "{label} input should reach its draft");
    }
}

#[test]
fn catalog_profile_form_paste_edits_database_and_role_name_fields() {
    let cases = [
        lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Database,
        ),
        lazydb::db::catalog_mutation::CatalogObjectType::LoginRole,
        lazydb::db::catalog_mutation::CatalogObjectType::Role,
    ];

    for object_type in cases {
        let mut app = App::new(Vec::new());
        let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
            lazydb::db::catalog_mutation::CatalogMutationMode::Create,
            lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
                profile_id: Uuid::nil(),
            },
            0,
            vec![],
        );
        assert!(editor.select_object_type(object_type));
        app.catalog_editor = Some(editor);
        app.overlay = Some(Overlay::CatalogEditor);

        let actions = map_paste("alpha 界 beta".into(), &app);
        assert_eq!(actions.len(), 1);
        for action in actions {
            app.update(action);
        }

        app.update(Action::CatalogEditorUndo);
        let undone_name = match app
            .catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
            .unwrap()
        {
            lazydb::model::catalog_editor::CatalogDraft::Database(draft) => draft.name.value(),
            lazydb::model::catalog_editor::CatalogDraft::Role(draft) => draft.name.value(),
            _ => panic!("database or role draft expected"),
        };
        assert_eq!(undone_name, "");
        app.update(Action::CatalogEditorRedo);

        let name = match app
            .catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
            .unwrap()
        {
            lazydb::model::catalog_editor::CatalogDraft::Database(draft) => draft.name.value(),
            lazydb::model::catalog_editor::CatalogDraft::Role(draft) => draft.name.value(),
            _ => panic!("database or role draft expected"),
        };
        assert_eq!(name, "alpha 界 beta");
    }
}

#[test]
fn catalog_profile_form_enter_preserves_preview_and_toggle_semantics() {
    let cases = [
        lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Database,
        ),
        lazydb::db::catalog_mutation::CatalogObjectType::LoginRole,
        lazydb::db::catalog_mutation::CatalogObjectType::Role,
    ];

    for object_type in cases {
        let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
            lazydb::db::catalog_mutation::CatalogMutationMode::Create,
            lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
                profile_id: Uuid::nil(),
            },
            0,
            vec![],
        );
        assert!(editor.select_object_type(object_type));
        let mut app = App::new(Vec::new());
        app.catalog_editor = Some(editor);
        app.overlay = Some(Overlay::CatalogEditor);
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::CatalogEditorPreview)
        );

        let toggle = if matches!(
            object_type,
            lazydb::db::catalog_mutation::CatalogObjectType::Catalog(_)
        ) {
            lazydb::model::catalog_editor::CatalogFormFocus::AllowConnections
        } else {
            lazydb::model::catalog_editor::CatalogFormFocus::Login
        };
        app.update(Action::CatalogEditorFocusFormField(toggle));
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::CatalogEditorToggleFocused)
        );

        app.update(Action::CatalogEditorFocusFormField(
            lazydb::model::catalog_editor::CatalogFormFocus::Review,
        ));
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::CatalogEditorPreview)
        );
        app.update(Action::CatalogEditorFocusFormField(
            lazydb::model::catalog_editor::CatalogFormFocus::Cancel,
        ));
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::CatalogEditorCancel)
        );
    }
}

#[test]
fn catalog_profile_form_paste_rejects_non_text_focus() {
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        0,
        vec![],
    );
    assert!(editor.select_object_type(lazydb::db::catalog_mutation::CatalogObjectType::LoginRole));
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(editor);
    app.overlay = Some(Overlay::CatalogEditor);

    for focus in [
        lazydb::model::catalog_editor::CatalogFormFocus::Login,
        lazydb::model::catalog_editor::CatalogFormFocus::Review,
        lazydb::model::catalog_editor::CatalogFormFocus::Cancel,
    ] {
        app.update(Action::CatalogEditorFocusFormField(focus));
        assert!(map_paste("must not be inserted".into(), &app).is_empty());
    }

    app.update(Action::CatalogEditorFocusFormField(
        lazydb::model::catalog_editor::CatalogFormFocus::Password,
    ));
    let actions = map_paste("safe synthetic".into(), &app);
    assert_eq!(actions.len(), 1);
    assert!(!format!("{:?}", actions[0]).contains("safe synthetic"));
    app.update(actions.into_iter().next().unwrap());
    let role = match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .unwrap()
    {
        lazydb::model::catalog_editor::CatalogDraft::Role(role) => role,
        _ => panic!("role draft expected"),
    };
    assert_eq!(role.password.as_ref().unwrap().expose(), "safe synthetic");
    assert!(!format!("{role:?}").contains("safe synthetic"));
}

#[test]
fn catalog_profile_form_text_field_reports_preview_on_enter() {
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        0,
        vec![],
    );
    assert!(
        editor.select_object_type(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Database
        ))
    );
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(editor);
    app.overlay = Some(Overlay::CatalogEditor);
    app.update(Action::CatalogEditorInsert('x'));

    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
}

#[test]
fn database_editor_toggle_key_is_routed_to_catalog_toggle() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        database_kind: None,
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Database,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Database(
            lazydb::model::catalog_editor::DatabaseDraft::new(""),
        )),
        baseline: None,
        plan: None,
        error: None,
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let mut keymap = Keymap::default();

    app.update(Action::CatalogEditorFocusFormField(
        lazydb::model::catalog_editor::CatalogFormFocus::AllowConnections,
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::CatalogEditorToggleFocused)
    );
}

#[test]
fn data_query_completion_keys_preempt_query_input_navigation() {
    let mut app = App::new(Vec::new());
    let mut tab = RelationTab::new("users");
    tab.query.capability = lazydb::model::data_query::DataQueryCapability::Relation;
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
fn focused_data_query_accepts_text_history_keys() {
    let mut app = App::new(Vec::new());
    let mut tab = RelationTab::new("users");
    tab.query.capability = lazydb::model::data_query::DataQueryCapability::Relation;
    tab.query.focus = Some(DataQueryInput::Where);
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(text_undo(), &app), Some(Action::DataQueryUndo));
    assert_eq!(keymap.map(text_redo(), &app), Some(Action::DataQueryRedo));
}

#[test]
fn focused_catalog_table_field_accepts_text_history_keys() {
    let app = table_editor_app();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(text_undo(), &app),
        Some(Action::CatalogEditorUndo)
    );
    assert_eq!(
        keymap.map(text_redo(), &app),
        Some(Action::CatalogEditorRedo)
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
fn catalog_drop_overlay_maps_option_navigation_and_ignores_text() {
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
        maintenance_database: None,
        delete_selected: false,
        busy: false,
        error: None,
    });
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogDropConfirm)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('y')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('Y')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CatalogDropCancel)
    );
    for code in [
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Left,
        KeyCode::Right,
    ] {
        assert_eq!(
            keymap.map(key(code), &app),
            Some(Action::ToggleCatalogDropFocus)
        );
    }
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
    assert_eq!(
        keymap.map(key(KeyCode::PageDown), &app),
        Some(Action::CatalogEditorPreviewScroll(10))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Home), &app),
        Some(Action::CatalogEditorPreviewHome)
    );
    assert_eq!(
        keymap.map(key(KeyCode::End), &app),
        Some(Action::CatalogEditorPreviewEnd)
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
        database_kind: None,
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
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
        database_kind: None,
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
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
                clickhouse_native: None,
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
    assert_eq!(keymap.map(key(KeyCode::Enter), &app), None);
}

#[test]
fn sequence_editor_form_owns_text_input_keys() {
    let mut app = App::new(vec![
        import_connection_url(":memory:", Some("test"))
            .unwrap()
            .profile,
    ]);
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        database_kind: None,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Sequence(
            lazydb::model::catalog_editor::SequenceDraft {
                name: "seq".into(),
                schema: "public".into(),
                owner: "owner".into(),
                comment: "".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: lazydb::db::catalog_mutation::SequenceBound::Unset.into(),
                max_value: lazydb::db::catalog_mutation::SequenceBound::Unset.into(),
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

    let draft = match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        lazydb::model::catalog_editor::CatalogDraft::Sequence(draft) => draft,
        _ => unreachable!(),
    };
    draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::MinValue;
    assert_eq!(
        keymap.map(key(KeyCode::Left), &app),
        Some(Action::CatalogEditorCycleChoice(-1))
    );
    assert_eq!(keymap.map(key(KeyCode::Char('x')), &app), None);

    let draft = match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        lazydb::model::catalog_editor::CatalogDraft::Sequence(draft) => draft,
        _ => unreachable!(),
    };
    draft.min_value = lazydb::db::catalog_mutation::SequenceBound::Value(String::new()).into();
    assert_eq!(
        keymap.map(key(KeyCode::Char('-')), &app),
        Some(Action::CatalogEditorInsert('-'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Left), &app),
        Some(Action::CatalogEditorMoveLeft)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char(' ')), &app),
        Some(Action::CatalogEditorCycleChoice(1))
    );
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

#[test]
fn help_sql_history_enter_opens_history_and_loads_first_page() {
    for focus in [Focus::Explorer, Focus::Editor, Focus::Results] {
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        app.focus = focus;
        let tab_count = app.tabs.len();
        let active_tab = app.active_tab;

        app.update(Action::ShowHelp);
        app.update(Action::HelpPaste("SQL execution history".into()));
        assert_eq!(
            app.help_selected_id(),
            Some(lazydb::help::HelpShortcutId::OpenSqlHistory),
            "history should be selected from {focus:?}"
        );

        let mut keymap = Keymap::default();
        let action = keymap.map(key(KeyCode::Enter), &app);
        assert_eq!(
            action,
            Some(Action::ExecuteHelpShortcut(
                lazydb::help::HelpShortcutId::OpenSqlHistory
            ))
        );
        let commands = app.update(action.unwrap());

        let Some(Overlay::SqlHistory(view)) = app.overlay.as_ref() else {
            panic!("expected SQL history overlay from {focus:?}");
        };
        assert_eq!(
            view.mode,
            lazydb::model::sql_history_view::SqlHistoryMode::Browse
        );
        assert!(view.loading);
        assert_ne!(view.overlay_id, Uuid::nil());
        assert!(view.query_generation > 0);
        assert_eq!(app.focus, Focus::Results);
        assert_eq!(app.tabs.len(), tab_count);
        assert_eq!(app.active_tab, active_tab);

        let [
            lazydb::action::Command::LoadSqlHistory {
                overlay_id,
                generation,
                request,
            },
        ] = commands.as_slice()
        else {
            panic!("expected exactly one SQL history load command");
        };
        assert_eq!(*overlay_id, view.overlay_id);
        assert_eq!(*generation, view.query_generation);
        assert!(request.cursor.is_none());
        assert!(request.limit > 0);
    }
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
fn editor_space_m_opens_notification_history_through_editor_effects() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Editor;
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    let mut keymap = Keymap::default();

    for character in [' ', 'm'] {
        let action = keymap
            .map(key(KeyCode::Char(character)), &app)
            .expect("editor key action");
        app.update(action);
    }

    assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));
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
fn relation_browse_y_copies_the_selected_cell() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('y')), &app),
        Some(Action::CopyGridCell)
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
    assert_eq!(
        keymap.map(key(KeyCode::F(6)), &app),
        Some(Action::OpenSqlEditorList)
    );
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('h')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Esc), &app), None);
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(ctrl('h'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Esc), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char(']')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
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
        database_kind: None,
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
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
fn data_query_paste_routes_relation_and_sql_fields() {
    use lazydb::model::{
        data_query::{DataQueryCapability, DataQueryInput},
        tab::{ConsoleTab, ResultView},
    };

    let payload = "status = 'café'\nAND active = true";
    for input in [DataQueryInput::Where, DataQueryInput::OrderBy] {
        let mut relation = RelationTab::new("users");
        relation.query.capability = DataQueryCapability::Relation;
        relation.query.focus = Some(input);
        let mut relation_app = App::new(Vec::new());
        relation_app.tabs.push(WorkspaceTab::Relation(relation));
        relation_app.active_tab = 1;
        relation_app.focus = Focus::Results;
        assert_eq!(
            map_paste(payload.to_owned(), &relation_app),
            vec![Action::DataQueryPaste(payload.to_owned())],
            "relation {input:?} input should receive the complete paste"
        );

        let mut console = ConsoleTab::new("query");
        console.result_view = ResultView::Data;
        console.query.capability = DataQueryCapability::Sql;
        console.query.focus = Some(input);
        let mut sql_app = App::new(Vec::new());
        sql_app.tabs.push(WorkspaceTab::Sql(console));
        sql_app.active_tab = 1;
        sql_app.focus = Focus::Results;
        assert_eq!(
            map_paste(payload.to_owned(), &sql_app),
            vec![Action::DataQueryPaste(payload.to_owned())],
            "SQL {input:?} input should receive the complete paste"
        );
    }
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
    assert_eq!(keymap.map(key(KeyCode::Char('n')), &app), None);
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

    app.explorer.normalized.selected = Some(ExplorerNodeId::EmptyProfiles);
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenExplorerAdd)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('e')), &app), None);
}

#[test]
fn empty_explorer_enter_and_add_open_connection_creation() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ExplorerOpenSelected)
    );
    app.update(Action::ExplorerOpenSelected);
    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));

    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenExplorerAdd)
    );
    app.update(Action::OpenExplorerAdd);
    assert!(
        matches!(app.overlay, Some(Overlay::ExplorerAdd(ref menu)) if menu.selected_kind() == Some(lazydb::model::explorer_add::ExplorerAddKind::Connection))
    );
    app.update(Action::ExplorerAddConfirm);
    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));
}

fn materialized_view_editor(
    mode: lazydb::db::catalog_mutation::CatalogMutationMode,
    focus: lazydb::model::catalog_editor::CatalogFormFocus,
) -> App {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        database_kind: None,
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
                    clickhouse_native: None,
                },
            ),
        ),
        baseline: None,
        plan: None,
        error: None,
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
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
        Some(Action::CatalogEditorToggleFocused)
    );
}

#[test]
fn catalog_editor_maps_only_focused_control_and_text_keys() {
    let mut app = materialized_view_editor(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::model::catalog_editor::CatalogFormFocus::WithData,
    );
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorToggleFocused)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('x')), &app), None);

    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft) => {
            draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::Review;
        }
        _ => unreachable!(),
    }
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorPreview)
    );
    assert_eq!(keymap.map(key(KeyCode::Char('x')), &app), None);
    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft) => {
            draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::Cancel;
        }
        _ => unreachable!(),
    }
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::CatalogEditorCancel)
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
        target: lazydb::model::transaction::DeferredTransactionTarget::Console(
            app.active_console().id,
        ),
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
fn relation_transaction_review_uses_ctrl_s_without_direct_rollback() {
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
        Some(Action::OpenTransactionControl)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
    assert_ne!(
        keymap.map(key(KeyCode::Char('c')), &app),
        Some(Action::OpenTransactionControl)
    );
}

#[test]
fn transaction_review_tab_cycles_preview_and_buttons() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = lazydb::model::transaction::TransactionState::Active;
    relation.transaction_review_sql = Some("SELECT 1;".into());
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.update(Action::OpenTransactionControl);
    let mut keymap = Keymap::default();

    for expected in [
        lazydb::model::transaction_review::TransactionReviewFocus::SqlPreview,
        lazydb::model::transaction_review::TransactionReviewFocus::Commit,
        lazydb::model::transaction_review::TransactionReviewFocus::Rollback,
        lazydb::model::transaction_review::TransactionReviewFocus::Cancel,
    ] {
        assert_eq!(
            keymap.map(key(KeyCode::Tab), &app),
            Some(Action::TransactionReviewFocusNext)
        );
        app.update(Action::TransactionReviewFocusNext);
        let Some(lazydb::model::workspace::Overlay::RelationTransactionConfirm(review)) =
            &app.overlay
        else {
            panic!("review closed")
        };
        assert_eq!(review.focus, expected);
    }
}

#[test]
fn transaction_review_preview_enter_does_not_confirm_and_escape_unwinds_search() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = lazydb::model::transaction::TransactionState::Active;
    relation.transaction_review_sql = Some("SELECT 1;\nSELECT 2;".into());
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.update(Action::OpenTransactionControl);
    app.update(Action::TransactionReviewFocusNext);

    let mut keymap = Keymap::default();
    let id = app.review_preview_session_id().unwrap();
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ReadOnlyEditorKey {
            session_id: id,
            event: key(KeyCode::Enter),
        })
    );
    app.update(Action::ReadOnlyEditorKey {
        session_id: id,
        event: key(KeyCode::Char('/')),
    });
    assert!(app.review_preview_has_pending_interaction());
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::ReadOnlyEditorKey {
            session_id: id,
            event: key(KeyCode::Esc),
        })
    );
    app.update(Action::ReadOnlyEditorKey {
        session_id: id,
        event: key(KeyCode::Esc),
    });
    assert!(!app.review_preview_has_pending_interaction());
    assert!(app.overlay.is_some());
}

#[test]
fn transaction_review_prompt_paste_is_owned_by_preview_session() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = lazydb::model::transaction::TransactionState::Active;
    relation.transaction_review_sql = Some("SELECT 1;".into());
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.update(Action::OpenTransactionControl);
    app.update(Action::TransactionReviewFocusNext);
    let id = app.review_preview_session_id().unwrap();
    app.update(Action::ReadOnlyEditorKey {
        session_id: id,
        event: key(KeyCode::Char('/')),
    });

    assert_eq!(
        lazydb::input::keymap::map_paste("users".into(), &app),
        vec![Action::TransactionReviewPromptPaste("users".into())]
    );
    app.update(Action::TransactionReviewPromptPaste("users".into()));
    assert!(app.review_preview_has_pending_interaction());
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::RelationTransactionConfirm(_))
    ));
}

#[test]
fn relation_help_executes_space_tc_transaction_control() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = lazydb::model::transaction::TransactionState::Active;
    relation.edit = Some(lazydb::model::relation_edit::RelationEditSession::from_rows(Vec::new()));
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;

    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("review and commit changes".into()));
    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::RelationCommit)
    );

    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::RelationCommit,
    ));
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::RelationTransactionConfirm(_))
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
    app.update(Action::NewConsole);
    app.focus = Focus::Editor;
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
            selected: 1,
            ..
        }) if candidates.len() == 2
            && candidates[1]
                .target()
                .is_some_and(|target| target.profile_id == profile_id)
    ));
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::CancelTargetSelector)
    );
}

#[test]
fn global_leader_opens_console_manager_and_current_console_target_selector() {
    let profile = profile("target");
    let profile_id = profile.id;
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    app.profiles.push(profile);
    app.focus = Focus::Results;
    let console_id = app.active_console().id;
    let mut keymap = Keymap::default();

    keymap.map(key(KeyCode::Char(' ')), &app);
    assert_eq!(
        keymap.map(key(KeyCode::Char('s')), &app),
        Some(Action::OpenSqlEditorList)
    );

    keymap.map(key(KeyCode::Char(' ')), &app);
    assert_eq!(
        keymap.map(key(KeyCode::Char('B')), &app),
        Some(Action::OpenConsoleTargetSelector { console_id })
    );
    app.update(Action::OpenConsoleTargetSelector { console_id });
    assert!(matches!(
        app.overlay,
        Some(Overlay::TargetSelector { ref candidates, .. })
            if candidates
                .iter()
                .any(|target| target.target().is_some_and(|target| target.profile_id == profile_id))
    ));
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

    let mut keymap = Keymap::default();
    assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('g')), &app),
        Some(Action::ExplorerSelectTarget(
            lazydb::model::explorer::ExplorerNodeTarget::First
        ))
    );

    for suffix in ['t', 'T'] {
        let mut keymap = Keymap::default();
        assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
        assert_eq!(keymap.map(key(KeyCode::Char(suffix)), &app), None);
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
fn editing_explorer_find_treats_n_and_shift_n_as_query_text() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_find();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::ExplorerFindInsert('n'))
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT), &app),
        Some(Action::ExplorerFindInsert('N'))
    );
}

#[test]
fn editing_explorer_find_maps_ctrl_w_and_filters_other_modifiers() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_find();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(control(KeyCode::Char('w')), &app),
        Some(Action::ExplorerFindDeletePreviousWord)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('w')), &app),
        Some(Action::ExplorerFindInsert('w'))
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT), &app),
        Some(Action::ExplorerFindInsert('W'))
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('u')), &app),
        Some(Action::ExplorerFindClear)
    );
    assert_eq!(keymap.map(control(KeyCode::Char('r')), &app), None);
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT), &app),
        None
    );
}

#[test]
fn editing_explorer_search_maps_ctrl_w_to_word_delete() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_search(None, 1);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(control(KeyCode::Char('w')), &app),
        Some(Action::ExplorerSearchDeletePreviousWord)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('w')), &app),
        Some(Action::ExplorerSearchInsert('w'))
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT), &app),
        Some(Action::ExplorerSearchInsert('W'))
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('u')), &app),
        Some(Action::ExplorerSearchClear)
    );
    assert_eq!(keymap.map(control(KeyCode::Char('r')), &app), None);
}

#[test]
fn confirmed_explorer_search_keeps_ctrl_w_window_binding() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_search(None, 1);
    app.explorer.confirm_search();
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(control(KeyCode::Char('w')), &app), None);
    assert_eq!(
        keymap.map(control(KeyCode::Char('w')), &app),
        Some(Action::FocusNext)
    );
}

#[test]
fn explorer_word_delete_actions_work_without_an_active_console() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    assert!(app.active_console_opt().is_none());

    app.explorer.open_search(None, 1);
    app.explorer
        .edit_search_input(|query| query.set("alpha beta"));
    assert!(
        app.update(Action::ExplorerSearchDeletePreviousWord)
            .is_empty()
    );
    assert_eq!(
        app.explorer.search.as_ref().unwrap().query.value(),
        "alpha "
    );

    app.explorer.search = None;
    app.explorer.open_find();
    app.explorer
        .edit_find_input(|query| query.set("alpha beta"));
    assert!(
        app.update(Action::ExplorerFindDeletePreviousWord)
            .is_empty()
    );
    assert_eq!(app.explorer.find.as_ref().unwrap().query.value(), "alpha ");
}

#[test]
fn confirmed_explorer_find_maps_shift_n_to_previous_match() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.explorer.open_find();
    app.explorer.confirm_find();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT), &app),
        Some(Action::ExplorerFindPrevious)
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
        KeyCode::Char('f'),
        KeyCode::Char('b'),
        KeyCode::Char('d'),
        KeyCode::Char('u'),
        KeyCode::PageUp,
        KeyCode::PageDown,
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
fn read_only_normal_panes_open_notification_history_from_space_m() {
    let mut ddl_app = App::new(Vec::new());
    ddl_app
        .tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    ddl_app.active_tab = 1;
    ddl_app.focus = Focus::Results;
    ddl_app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));

    let mut output_app = App::new(Vec::new());
    output_app.focus = Focus::Results;
    output_app.active_console_mut().result_view = lazydb::model::tab::ResultView::Output;

    let mut plan_app = App::new(Vec::new());
    plan_app.focus = Focus::Results;
    plan_app.active_console_mut().result_view = lazydb::model::tab::ResultView::Plan;

    for mut app in [ddl_app, output_app, plan_app] {
        let mut keymap = Keymap::default();
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
        let action = keymap
            .map(key(KeyCode::Char('m')), &app)
            .expect("notification history action");
        app.update(action);
        assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));
    }
}

#[test]
fn empty_notification_history_supports_search_and_close() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    for code in [KeyCode::Char(' '), KeyCode::Char('m')] {
        if let Some(action) = keymap.map(key(code), &app) {
            app.update(action);
        }
    }
    assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));

    for code in [KeyCode::Char('/'), KeyCode::Char('x'), KeyCode::Esc] {
        let action = keymap.map(key(code), &app).expect("history action");
        app.update(action);
    }
    assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));

    let action = keymap
        .map(key(KeyCode::Char('q')), &app)
        .expect("close action");
    app.update(action);
    assert!(app.overlay.is_none());
}

#[test]
fn clearing_notification_history_keeps_empty_history_reopenable() {
    let mut app = App::new(Vec::new());
    app.notifications.push(
        NotificationLevel::Info,
        "Title",
        "Body",
        std::time::Instant::now(),
    );
    app.update(Action::OpenNotificationHistory);
    app.update(Action::NotificationHistoryClear);
    app.update(Action::NotificationHistoryClearConfirm);
    assert!(app.notifications.history().next().is_none());

    app.update(Action::DismissOverlay);
    app.update(Action::OpenNotificationHistory);
    assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));
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
    assert_eq!(keymap.map(ctrl('s'), &app), None);
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), &app,),
        None
    );
    assert_eq!(
        keymap.map(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT,),
            &app,
        ),
        None
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
    assert_eq!(keymap.map(text_undo(), &app), Some(Action::ProfileUndo));
    assert_eq!(keymap.map(text_redo(), &app), Some(Action::ProfileRedo));
    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Url;
    assert_eq!(keymap.map(text_undo(), &app), Some(Action::ProfileUndo));
    app.profile_manager.as_mut().unwrap().selected_field = ProfileField::Password;
    assert_eq!(keymap.map(text_redo(), &app), Some(Action::ProfileRedo));
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
        Some(Action::ProfileSave { connect: true })
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
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::ProfileSave { connect: true })
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);

    for field in [ProfileField::SslMode, ProfileField::Environment] {
        app.profile_manager.as_mut().unwrap().selected_field = field;
        for (code, expected) in [
            (KeyCode::Left, Action::ProfileCycle(-1)),
            (KeyCode::Char('h'), Action::ProfileCycle(-1)),
            (KeyCode::Right, Action::ProfileCycle(1)),
            (KeyCode::Char('l'), Action::ProfileCycle(1)),
            (KeyCode::Enter, Action::ProfileSave { connect: true }),
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
        (ProfileField::Save, Action::ProfileSave { connect: true }),
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
        Some(Action::ProfileCancelDelete)
    );
    assert!(app.update(Action::ToggleProfileDeleteFocus).is_empty());
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

// ---------------------------------------------------------------------------
// T5 (S3): the field-by-field view inside the MongoDB document modal
// ---------------------------------------------------------------------------

/// An app whose active relation tab holds an open MongoDB document modal.
fn document_modal_app() -> App {
    use lazydb::db::mongodb::types::{MongoDocument, MongoNamespace};
    use lazydb::model::relation_edit::{
        DocumentEditorState, RelationEditSession, RelationGridMode,
    };

    let namespace = MongoNamespace::collection("app", "bikes");
    let bson = mongodb::bson::doc! { "_id": 1_i32, "name": "Ruedas Pro", "price": 1200_i64 };
    let document = MongoDocument {
        raw: mongodb::bson::raw::RawDocumentBuf::from_bytes(
            mongodb::bson::to_vec(&bson).expect("encodes"),
        )
        .expect("raw"),
        bson,
        editable: true,
    };
    let mut state = DocumentEditorState::new(0, &namespace, &document).expect("the document opens");
    // The modal opens on the field list; these tests pin the keys of both
    // views, so they start from the EJSON view explicitly.
    state.view = lazydb::model::relation_edit::DocumentEditorView::Ejson;

    let mut app = App::new(Vec::new());
    let mut relation = RelationTab::new("bikes");
    let mut edit = RelationEditSession::from_rows(vec![Vec::new(); 1]);
    edit.mode = RelationGridMode::EditDocument(Box::new(state));
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    app
}

#[test]
fn the_document_modal_toggles_its_two_views_with_tab() {
    let mut app = document_modal_app();
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::RelationEditDocumentToggleView),
        "Tab switches from EJSON to the field list"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RelationEditCancel),
        "Esc still closes the modal"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RelationEditInsert('\n')),
        "Enter still inserts a newline in EJSON"
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('s')), &app),
        Some(Action::RelationEditConfirm),
        "Ctrl-S still saves the document"
    );

    app.update(Action::RelationEditDocumentToggleView);

    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::RelationEditDocumentToggleView),
        "Tab switches back to EJSON"
    );
    for (event, expected) in [
        (
            key(KeyCode::Char('j')),
            Action::RelationEditDocumentFieldMove(1),
        ),
        (key(KeyCode::Down), Action::RelationEditDocumentFieldMove(1)),
        (
            key(KeyCode::Char('k')),
            Action::RelationEditDocumentFieldMove(-1),
        ),
        (key(KeyCode::Up), Action::RelationEditDocumentFieldMove(-1)),
    ] {
        assert_eq!(keymap.map(event, &app), Some(expected));
    }
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RelationEditDocumentFieldEdit),
        "Enter edits the selected field"
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('s')), &app),
        Some(Action::RelationEditConfirm),
        "Ctrl-S in the list still saves the document"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RelationEditCancel),
        "Esc in the list closes the modal"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        None,
        "typing in the list never edits the EJSON text"
    );
}

#[test]
fn a_field_editor_owns_apply_cancel_and_typing() {
    let mut app = document_modal_app();
    app.update(Action::RelationEditDocumentToggleView);
    app.update(Action::RelationEditDocumentFieldMove(1));
    app.update(Action::RelationEditDocumentFieldEdit);
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(control(KeyCode::Char('s')), &app),
        Some(Action::RelationEditDocumentFieldApply),
        "Ctrl-S applies the field"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RelationEditDocumentFieldApply),
        "Enter applies the single-line field value"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RelationEditDocumentFieldCancel),
        "Esc leaves the field editor instead of closing the modal"
    );
    for (event, expected) in [
        (key(KeyCode::Char('x')), Action::RelationEditInsert('x')),
        (key(KeyCode::Backspace), Action::RelationEditBackspace),
        (
            control(KeyCode::Char('w')),
            Action::RelationEditDeletePreviousWord,
        ),
        (key(KeyCode::Left), Action::RelationEditMoveLeft),
    ] {
        assert_eq!(keymap.map(event, &app), Some(expected));
    }
}

#[test]
fn the_sql_cell_editor_keys_are_unchanged_by_the_document_field_view() {
    use lazydb::model::relation_edit::{CellEditorState, RelationEditSession, RelationGridMode};

    let mut app = App::new(Vec::new());
    let mut relation = RelationTab::new("users");
    let mut edit = RelationEditSession::from_rows(vec![vec![lazydb::db::value::CellValue::Text(
        "old".into(),
    )]]);
    edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
        row: 0,
        column: 0,
        input: lazydb::model::cell_editor::CellEditorBuffer::default(),
        error: None,
    }));
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RelationEditConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RelationEditCancel)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('x')), &app),
        Some(Action::RelationEditInsert('x'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        None,
        "Tab stays unbound in the SQL cell editor"
    );
}

#[test]
fn the_sql_json_cell_editor_keys_are_unchanged_by_the_document_field_view() {
    use lazydb::model::relation_edit::{CellEditorState, RelationEditSession, RelationGridMode};

    let mut app = App::new(Vec::new());
    let mut relation = RelationTab::new("users");
    let mut edit =
        RelationEditSession::from_rows(vec![vec![lazydb::db::value::CellValue::Text("{}".into())]]);
    edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
        row: 0,
        column: 0,
        input: lazydb::model::cell_editor::CellEditorBuffer {
            presence: lazydb::model::cell_editor::CellEditorPresence::Value,
            content: lazydb::model::cell_editor::CellEditorContent::Typed {
                kind: lazydb::model::cell_editor::CellEditorKind::Json,
                draft: lazydb::model::cell_editor::TypedDraft::Json(
                    lazydb::model::cell_editor::JsonBuffer::new("{}"),
                ),
            },
            ..Default::default()
        },
        error: None,
    }));
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(control(KeyCode::Char('s')), &app),
        Some(Action::RelationEditConfirm)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Enter), &app),
        Some(Action::RelationEditInsert('\n'))
    );
    assert_eq!(
        keymap.map(control(KeyCode::Char('f')), &app),
        Some(Action::RelationEditJsonFormat)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Down), &app),
        Some(Action::RelationEditJsonMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::RelationEditCancel)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        None,
        "Tab stays unbound in the SQL JSON cell editor"
    );
}

#[test]
fn m_and_u_open_monitor_and_users_from_a_mongodb_database_row() {
    use lazydb::db::mongodb::types::MongoExplorerNodeId;
    use lazydb::model::explorer::{ProfilePlacement, ProfileProvenance};
    use lazydb::profile::DatabaseKind;

    let profile_id = Uuid::from_u128(4242);
    let mut app = App::new(Vec::new());
    app.explorer.normalized.add_profile_with_placement(
        profile_id,
        "MongoDemo".into(),
        DatabaseKind::MongoDb,
        "127.0.0.1".into(),
        ProfileProvenance::Saved,
        ProfilePlacement::Global,
    );
    let profile_node = ExplorerNodeId::Profile(profile_id);
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .mongodb_databases = vec!["lazydb_demo".into()];
    app.explorer.normalized.expanded.insert(profile_node);
    let database_node = ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Database("lazydb_demo".into()),
    };
    assert!(app.explorer.select_id(database_node));
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('m')), &app),
        Some(Action::OpenMongoNode(MongoExplorerNodeId::Monitor(
            "lazydb_demo".into()
        ))),
        "m opens the database monitor"
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('u')), &app),
        Some(Action::OpenMongoNode(MongoExplorerNodeId::Users(
            "lazydb_demo".into()
        ))),
        "u opens users and roles"
    );
}
