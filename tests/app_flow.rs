use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::{Action, Command},
    app::App,
    model::{
        tab::CompletionPopup,
        transaction::{TransactionMode, TransactionState},
        workspace::{ConnectionStatus, Overlay},
    },
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    profile::import_connection_url,
    runtime::Runtime,
    sql::{CompletionCandidate, CompletionKind, CompletionScore, TextRange},
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};
use uuid::Uuid;

#[test]
fn typing_refreshes_an_open_completion_without_flicker() {
    let mut app = App::new(Vec::new());
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    assert!(app.active_console().completion.is_some());

    let commands = app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::NONE,
    )));

    let after = app.active_console().completion.as_ref().unwrap();
    assert_eq!(app.active_editor_text().unwrap(), "sele");
    assert!(!after.candidates.is_empty());
    assert!(
        !commands
            .iter()
            .any(|command| { matches!(command, lazydb::action::Command::ScheduleCompletion(_)) }),
        "commands: {commands:?}"
    );
}

#[test]
fn editor_text_changes_schedule_diagnostics_and_clear_stale_results() {
    let mut app = App::new(Vec::new());
    app.active_console_mut()
        .semantic_diagnostics
        .push(lazydb::sql::SqlDiagnostic {
            range: TextRange::new(0, 1),
            message: "stale".into(),
            code: "test",
        });

    let commands = app.update(Action::ReplaceEditor("select 1".into()));

    assert!(app.active_console().semantic_diagnostics.is_empty());
    assert!(
        commands
            .iter()
            .any(|command| { matches!(command, Command::ScheduleDiagnostics(_)) })
    );
}

#[test]
fn catalog_unavailable_relation_error_is_user_facing_and_terminal_safe() {
    let mut app = App::new(Vec::new());
    app.update(Action::RelationFailed {
        request: lazydb::model::relation::RelationRequest {
            tab_id: Uuid::nil(),
            console_id: Uuid::nil(),
            tab_generation: 0,
            request_id: 0,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id: Uuid::nil(),
                generation: 0,
            },
            relation: lazydb::model::relation::RelationKey {
                profile_id: Uuid::nil(),
                object_id: lazydb::db::catalog::CatalogId::new(
                    Uuid::nil(),
                    lazydb::db::catalog::CatalogKind::Table,
                    ["users"],
                ),
            },
            kind: lazydb::model::relation::RelationRequestKind::Preview,
            scope: lazydb::profile::CatalogScope::for_profile(
                lazydb::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
            options: Default::default(),
            page: lazydb::model::pagination::PageRequest::first(Default::default()),
        },
        message: "relation is not present in the active catalog snapshot\nsecret".into(),
    });
    let notification = app
        .notifications
        .history()
        .next()
        .expect("unavailable object notification");
    assert_eq!(
        notification.body,
        "Object is no longer available; refresh the catalog"
    );
    assert!(!notification.body.contains("active catalog snapshot"));
    assert!(!notification.body.contains("secret"));
}

#[test]
fn typing_space_after_a_statement_closes_completion_without_scheduling() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("select * from users;".into()));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: "users".into(),
            insert_text: "users".into(),
            kind: CompletionKind::Table,
            detail: None,
            replace: TextRange::new(19, 19),
            score: CompletionScore {
                context: 1,
                name_match: 1,
                schema: 0,
            },
        }],
        selected: 0,
    });
    assert!(app.active_console().completion.is_some());

    let commands = app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(' '),
        KeyModifiers::NONE,
    )));

    assert!(app.active_console().completion.is_none());
    assert!(
        !commands
            .iter()
            .any(|command| { matches!(command, lazydb::action::Command::ScheduleCompletion(_)) })
    );
}

#[test]
fn empty_editor_does_not_schedule_completion_after_entering_insert_mode() {
    let mut app = App::new(Vec::new());
    let commands = app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));

    assert!(app.active_console().completion.is_none());
    assert!(
        !commands
            .iter()
            .any(|command| { matches!(command, lazydb::action::Command::ScheduleCompletion(_)) }),
        "commands: {commands:?}"
    );
}

#[test]
fn insert_escape_closes_completion_and_exits_insert_mode() {
    let mut app = App::new(Vec::new());
    let original = app.active_editor_text().unwrap();
    app.active_console_mut().completion = Some(CompletionPopup::default());
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    assert!(app.active_console().completion.is_none());
    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );
    assert_eq!(app.active_editor_text().unwrap(), original);
}

#[test]
fn editor_copy_actions_emit_complete_clipboard_payloads() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("SELECT 1;".into()));

    let commands = app.update(Action::CopyEditorBuffer);
    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload == &lazydb::clipboard::ClipboardPayload {
            text: "SELECT 1;".into(),
            description: "SQL buffer".into(),
            sensitive: false,
        }
    ));
    let commands = app.update(Action::CopyEditorYank("SELECT".into()));
    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload == &lazydb::clipboard::ClipboardPayload {
            text: "SELECT".into(),
            description: "SQL selection: 6 chars".into(),
            sensitive: false,
        }
    ));

    let session_id = app.active_console().id;
    let revision = app.active_editor_revision();
    let commands = app.update(Action::CopyEditorSelection {
        session_id,
        start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
        end: lazydb::model::editor::EditorPosition { line: 0, column: 5 },
        revision,
    });
    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload == &lazydb::clipboard::ClipboardPayload {
            text: "SELECT".into(),
            description: "Text selection: 6 chars".into(),
            sensitive: false,
        }
    ));

    app.update(Action::ReplaceEditor("changed".into()));
    assert!(
        app.update(Action::CopyEditorSelection {
            session_id,
            start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
            end: lazydb::model::editor::EditorPosition { line: 0, column: 5 },
            revision,
        })
        .is_empty()
    );
}

#[test]
fn completing_mouse_selection_copies_editor_text_without_mutating_revision() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("SELECT 1;".into()));
    let session_id = app.active_console().id;
    let revision = app.active_editor_revision();

    let commands = app.update(Action::CompleteMouseTextSelection {
        source: lazydb::ui::text_selection::TextGestureSource::Editor,
        session_id,
        start: lazydb::model::editor::EditorPosition { line: 0, column: 1 },
        end: lazydb::model::editor::EditorPosition { line: 0, column: 5 },
        revision,
    });

    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload.text == "ELECT"
    ));
    assert_eq!(app.active_editor_revision(), revision);
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 4,
        })
        .unwrap();
    assert!(snapshot.selections.is_empty());
}

#[test]
fn clipboard_failure_keeps_selection_usable_without_echoing_payload() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("secret selection".into()));
    let session_id = app.active_console().id;
    let revision = app.active_editor_revision();

    app.update(Action::ClipboardWriteFailed {
        message: "Clipboard unavailable".into(),
    });

    let notification = app.notifications.history().next().unwrap();
    assert_eq!(notification.title, "Clipboard");
    assert_eq!(notification.body, "Clipboard unavailable");
    assert!(!notification.body.contains("secret selection"));

    let commands = app.update(Action::CopyEditorSelection {
        session_id,
        start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
        end: lazydb::model::editor::EditorPosition { line: 0, column: 5 },
        revision,
    });
    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload.text == "secret"
    ));
}

#[test]
fn text_detail_selection_and_copy_keep_the_selection_and_reject_stale_sources() {
    let mut app = App::new(Vec::new());
    let source_session_id = app.active_console().id;
    let source_revision = app.active_editor_revision();
    let detail = lazydb::model::text_detail::TextDetailRequest::new(
        "VALUE",
        source_session_id,
        source_revision,
        "safe display",
        "complete\nvalue",
        None,
    );

    app.update(Action::OpenTextDetail(detail));
    let (detail_session_id, detail_revision) = match app.overlay.as_ref() {
        Some(Overlay::TextDetail(view)) => (view.session_id, view.revision),
        other => panic!("expected text detail, got {other:?}"),
    };
    let commands = app.update(Action::CompleteMouseTextSelection {
        source: lazydb::ui::text_selection::TextGestureSource::TextDetail,
        session_id: detail_session_id,
        start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
        end: lazydb::model::editor::EditorPosition { line: 0, column: 3 },
        revision: detail_revision,
    });
    assert!(
        matches!(commands.as_slice(), [Command::WriteClipboard(payload)] if payload.text == "safe")
    );
    assert!(matches!(app.overlay, Some(Overlay::TextDetail(_))));

    let commands = app.update(Action::CopyTextDetailAll {
        session_id: detail_session_id,
    });
    assert!(
        matches!(commands.as_slice(), [Command::WriteClipboard(payload)] if payload.text == "complete\nvalue")
    );

    app.update(Action::ReplaceEditor("changed".into()));
    assert!(
        app.update(Action::CompleteMouseTextSelection {
            source: lazydb::ui::text_selection::TextGestureSource::TextDetail,
            session_id: detail_session_id,
            start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
            end: lazydb::model::editor::EditorPosition { line: 0, column: 3 },
            revision: detail_revision,
        })
        .is_empty()
    );
}

#[test]
fn completing_mouse_selection_updates_text_detail_before_copying() {
    let mut app = App::new(Vec::new());
    let source_session_id = app.active_console().id;
    let source_revision = app.active_editor_revision();
    app.update(Action::OpenTextDetail(
        lazydb::model::text_detail::TextDetailRequest::new(
            "VALUE",
            source_session_id,
            source_revision,
            "one two",
            "complete\nvalue",
            None,
        ),
    ));
    let (session_id, revision) = match app.overlay.as_ref() {
        Some(Overlay::TextDetail(view)) => (view.session_id, view.revision),
        _ => panic!("detail was not opened"),
    };

    let commands = app.update(Action::CompleteMouseTextSelection {
        source: lazydb::ui::text_selection::TextGestureSource::TextDetail,
        session_id,
        start: lazydb::model::editor::EditorPosition { line: 0, column: 0 },
        end: lazydb::model::editor::EditorPosition { line: 0, column: 2 },
        revision,
    });

    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(payload)] if payload.text == "one"
    ));
}

#[test]
fn text_detail_escape_closes_to_return_overlay_and_copy_button_preserves_selection() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::NotificationHistory(Default::default()));
    let return_overlay = app.overlay.clone().map(Box::new);
    let detail = lazydb::model::text_detail::TextDetailRequest::new(
        "DETAIL",
        app.active_console().id,
        app.active_editor_revision(),
        "one two",
        "one two",
        return_overlay,
    );
    app.update(Action::OpenTextDetail(detail));
    let session_id = match app.overlay.as_ref() {
        Some(Overlay::TextDetail(view)) => view.session_id,
        _ => panic!("detail was not opened"),
    };
    app.update(Action::CopyTextDetailAll { session_id });
    assert!(matches!(app.overlay, Some(Overlay::TextDetail(_))));
    app.update(Action::CloseTextDetail);
    assert!(matches!(app.overlay, Some(Overlay::NotificationHistory(_))));
}

fn editor_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.update(Action::EditorKey(KeyEvent::new(code, modifiers)));
}

fn mapped_editor_key(app: &mut App, keymap: &mut lazydb::input::keymap::Keymap, code: KeyCode) {
    let action = keymap
        .map(KeyEvent::new(code, KeyModifiers::NONE), app)
        .expect("editor key should be routed through the keymap");
    app.update(action);
}

#[test]
fn normal_mode_motions_do_not_insert_literal_keys_through_app() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one two\nthree".into()));

    for code in [
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
        KeyCode::Char('w'),
        KeyCode::Char('b'),
        KeyCode::Char('e'),
        KeyCode::Char('0'),
        KeyCode::Char('$'),
        KeyCode::Char('G'),
    ] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }

    assert_eq!(app.active_editor_text().unwrap(), "one two\nthree");
    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );
}

#[test]
fn normal_mode_gg_moves_cursor_to_first_line_through_keymap() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one\ntwo\nthree".into()));
    let mut keymap = lazydb::input::keymap::Keymap::default();

    mapped_editor_key(&mut app, &mut keymap, KeyCode::Char('G'));
    assert_eq!(
        app.active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 10,
        })
        .unwrap()
        .cursor
        .line,
        2
    );

    mapped_editor_key(&mut app, &mut keymap, KeyCode::Char('g'));
    mapped_editor_key(&mut app, &mut keymap, KeyCode::Char('g'));

    assert_eq!(
        app.active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 10,
        })
        .unwrap()
        .cursor
        .line,
        0
    );
}

#[test]
fn normal_mode_delete_line_does_not_reopen_completion() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("select 1\nselect 2".into()));
    app.active_console_mut().completion = Some(CompletionPopup::default());
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    for code in [KeyCode::Char('d'), KeyCode::Char('d')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }

    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );
    assert!(app.active_console().completion.is_none());
}

#[test]
fn space_tt_toggles_transaction_mode_through_app_pipeline() {
    let mut app = App::new(Vec::new());
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    for expected in [TransactionMode::Manual, TransactionMode::Auto] {
        for code in [KeyCode::Char(' '), KeyCode::Char('t'), KeyCode::Char('t')] {
            editor_key(&mut app, code, KeyModifiers::NONE);
        }
        assert_eq!(app.active_console().transaction_mode, expected);
    }
}

#[test]
fn space_tt_requires_exit_confirmation_for_active_manual_transaction() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    for code in [KeyCode::Char(' '), KeyCode::Char('t'), KeyCode::Char('t')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }

    assert_eq!(
        app.active_console().transaction_mode,
        TransactionMode::Manual
    );
    assert!(matches!(
        app.overlay,
        Some(Overlay::TransactionExitConfirm { .. })
    ));
}

#[test]
fn transaction_header_activation_preserves_active_manual_transaction() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;

    app.update(Action::ActivateEditorTransaction);

    assert_eq!(
        app.active_console().transaction_mode,
        TransactionMode::Manual
    );
    assert!(matches!(app.overlay, Some(Overlay::TransactionMenu { .. })));
}

#[test]
fn transaction_header_activation_does_not_clear_unknown_outcome() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::OutcomeUnknown;

    app.update(Action::ActivateEditorTransaction);

    assert_eq!(
        app.active_console().transaction_state,
        TransactionState::OutcomeUnknown
    );
    assert!(matches!(app.overlay, Some(Overlay::TransactionMenu { .. })));
}

#[test]
fn space_tc_opens_transaction_panel_and_space_tr_is_unused() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    for code in [KeyCode::Char(' '), KeyCode::Char('t'), KeyCode::Char('c')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    assert!(matches!(
        app.overlay,
        Some(Overlay::TransactionExitConfirm { .. })
    ));

    app.update(Action::CancelTransactionExit);
    for code in [KeyCode::Char(' '), KeyCode::Char('t'), KeyCode::Char('r')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    assert!(app.overlay.is_none());
    assert_eq!(
        app.active_console().transaction_state,
        TransactionState::Active
    );
}

#[test]
fn transaction_panel_enter_confirms_the_selected_choice() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;

    app.update(Action::OpenTransactionControl);
    app.update(Action::ConfirmTransactionExit);

    assert!(matches!(
        app.overlay,
        Some(Overlay::TransactionExitConfirm {
            choice: lazydb::model::transaction::TransactionExitChoice::Rollback,
            ..
        })
    ));
}

#[test]
fn relation_space_tc_no_longer_opens_the_transaction_panel() {
    let mut app = App::new(Vec::new());
    let mut relation = lazydb::model::relation::RelationTab::new("users");
    relation.transaction_state = TransactionState::Active;
    app.tabs
        .push(lazydb::model::tab::WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = lazydb::model::workspace::Focus::Results;

    let mut keymap = lazydb::input::keymap::Keymap::default();
    for code in [KeyCode::Char(' '), KeyCode::Char('t'), KeyCode::Char('c')] {
        if let Some(action) = keymap.map(KeyEvent::new(code, KeyModifiers::NONE), &app) {
            app.update(action);
        }
    }

    assert!(!matches!(
        app.overlay,
        Some(Overlay::RelationTransactionConfirm(_))
    ));
}

#[test]
fn vim_operator_text_object_visual_and_undo_sequences_use_app_pipeline() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one two three".into()));

    for code in [KeyCode::Char('c'), KeyCode::Char('i'), KeyCode::Char('w')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    for code in [KeyCode::Char('X'), KeyCode::Esc] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    assert_eq!(app.active_editor_text().unwrap(), "X two three");

    editor_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );

    editor_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
    assert_eq!(app.active_editor_text().unwrap(), "one two three");
    editor_key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(app.active_editor_text().unwrap(), "X two three");
}

#[test]
fn accepting_completion_places_cursor_after_inserted_text() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    assert!(app.active_console().completion.is_some());

    let commands = app.update(Action::CompletionAccept);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
    assert!(app.active_console().completion.is_none());
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, lazydb::action::Command::ScheduleCompletion(_)))
    );
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 5,
        })
        .unwrap();
    assert_eq!(
        snapshot.cursor,
        lazydb::model::editor::EditorPosition { line: 0, column: 7 }
    );

    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
    assert_eq!(app.active_editor_text().unwrap(), "");
    editor_key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
}

#[test]
fn accepting_completion_before_existing_whitespace_reuses_ascii_space() {
    for prefix in ["", "select 1;\n"] {
        for suffix in [" ", "  "] {
            let mut app = connected_completion_app();
            app.update(Action::ReplaceEditor(format!("{suffix}select 2;")));
            editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
            app.update(Action::EditorPaste(format!("{prefix}sel")));
            assert_eq!(
                app.active_editor_text().unwrap(),
                format!("{prefix}sel{suffix}select 2;")
            );
            assert_completion_cursor(&app, usize::from(!prefix.is_empty()), 3);
            app.update(Action::CompletionExplicit);
            select_completion(&mut app, "SELECT");

            let commands = app.update(Action::CompletionAccept);

            assert_eq!(
                app.active_editor_text().unwrap(),
                format!("{prefix}SELECT{suffix}select 2;")
            );
            assert_completion_cursor(&app, usize::from(!prefix.is_empty()), 7);
            assert!(app.active_console().completion.is_none());
            assert!(
                app.active_console().completion_request.is_none(),
                "prefix={prefix:?}, suffix={suffix:?}: {commands:?}"
            );
            assert!(
                !commands
                    .iter()
                    .any(|command| matches!(command, Command::ScheduleCompletion(_)))
            );
        }
    }
}

#[test]
fn accepting_completion_before_newline_or_tab_keeps_cursor_before_suffix() {
    for suffix in ["\n", "\t"] {
        let mut app = connected_completion_app();
        app.update(Action::ReplaceEditor(format!("{suffix}select 2;")));
        editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        app.update(Action::EditorPaste("sel".into()));
        app.update(Action::CompletionExplicit);
        select_completion(&mut app, "SELECT");

        app.update(Action::CompletionAccept);

        assert_eq!(
            app.active_editor_text().unwrap(),
            format!("SELECT{suffix}select 2;")
        );
        assert_completion_cursor(&app, 0, 6);
    }
}

fn connected_completion_app() -> App {
    let profile = import_connection_url("postgres://localhost/app", Some("completion"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::NewConsole);
    let generation = match app.update(Action::RequestConnect(profile.id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "17".into(),
            database: "app".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    assert!(app.connection.active_identity().is_some());
    app
}

fn select_completion(app: &mut App, insert_text: &str) {
    let popup = app.active_console_mut().completion.as_mut().unwrap();
    popup.selected = popup
        .candidates
        .iter()
        .position(|candidate| candidate.insert_text == insert_text)
        .unwrap_or_else(|| panic!("missing {insert_text:?}: {:?}", popup.candidates));
}

fn assert_completion_cursor(app: &App, line: usize, column: usize) {
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 5,
        })
        .unwrap();
    assert_eq!(
        snapshot.cursor,
        lazydb::model::editor::EditorPosition { line, column }
    );
}

#[test]
fn accepting_completion_does_not_add_space_before_ddl_punctuation() {
    for (text, replacement, insert_text) in [
        ("CREATE TABLE t (id IN)", TextRange::new(19, 21), "INTEGER"),
        ("REFERENCES users (i)", TextRange::new(18, 19), "id"),
        (
            "CREATE INDEX ix ON users (e, id)",
            TextRange::new(26, 27),
            "email",
        ),
        ("DROP TABLE us;", TextRange::new(11, 13), "users"),
    ] {
        let mut app = connected_completion_app();
        app.update(Action::ReplaceEditor(text.to_owned()));
        editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        app.active_console_mut().completion = Some(CompletionPopup {
            candidates: vec![CompletionCandidate {
                label: insert_text.to_owned(),
                insert_text: insert_text.to_owned(),
                kind: CompletionKind::Keyword,
                detail: None,
                replace: replacement,
                score: CompletionScore {
                    context: 0,
                    name_match: 0,
                    schema: 0,
                },
            }],
            selected: 0,
        });
        let commands = app.update(Action::CompletionAccept);
        let expected = format!(
            "{}{}{}",
            &text[..replacement.start],
            insert_text,
            &text[replacement.end..]
        );
        assert_eq!(app.active_editor_text().unwrap(), expected);
        assert!(app.active_console().completion.is_none());
        assert!(app.active_console().completion_request.is_none());
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::ScheduleCompletion(_)))
        );
    }
}

#[test]
fn accepting_completion_preserves_ordering_continuation() {
    for suffix in ["", " "] {
        let mut app = connected_completion_app();
        app.update(Action::ReplaceEditor(suffix.into()));
        editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        app.update(Action::EditorPaste("select 1 order by 1 des".into()));
        app.update(Action::CompletionExplicit);
        select_completion(&mut app, "DESC");

        let commands = app.update(Action::CompletionAccept);

        assert_eq!(
            app.active_editor_text().unwrap(),
            "select 1 order by 1 DESC "
        );
        assert!(app.active_console().completion.is_none());
        assert_completion_cursor(&app, 0, "select 1 order by 1 DESC ".len());
        let key = commands
            .iter()
            .find_map(|command| match command {
                Command::ScheduleCompletion(key) => Some(key.clone()),
                _ => None,
            })
            .expect("next ordering position must be scheduled");
        assert!(app.active_console().completion_request.is_some());
        app.update(Action::CompletionDue(key));
        let candidates = &app.active_console().completion.as_ref().unwrap().candidates;
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.insert_text == "NULLS FIRST")
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.insert_text == "NULLS LAST")
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.insert_text == "DESC")
        );
    }
}

#[test]
fn accepting_completion_does_not_suppress_later_input_or_explicit_requests() {
    for code in [KeyCode::Backspace, KeyCode::Char('x')] {
        let mut app = connected_completion_app();
        app.update(Action::ReplaceEditor(" ".into()));
        editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        app.update(Action::EditorPaste("sel".into()));
        app.update(Action::CompletionExplicit);
        select_completion(&mut app, "SELECT");
        app.update(Action::CompletionAccept);
        assert!(app.active_console().completion_request.is_none());

        app.update(Action::CompletionExplicit);
        // The cursor now moves past the reused space, so the next explicit
        // request is for the expression after SELECT rather than SELECT itself.
        assert!(
            app.active_console()
                .completion_request
                .as_ref()
                .unwrap()
                .explicit
        );
        app.update(Action::CompletionDismiss);
        let commands = app.update(Action::EditorKey(KeyEvent::new(code, KeyModifiers::NONE)));
        let key = commands
            .iter()
            .find_map(|command| match command {
                Command::ScheduleCompletion(key) => Some(key.clone()),
                _ => None,
            })
            .expect("later editing must schedule completion");
        let expected = if code == KeyCode::Backspace {
            "SELECT"
        } else {
            "SELECT x"
        };
        assert_eq!(app.active_editor_text().unwrap(), expected);
        let request = app.active_console().completion_request.as_ref().unwrap();
        assert_eq!(request.revision, app.active_editor_revision());
        assert_eq!(request.cursor, expected.len());
        app.update(Action::CompletionDue(key));
        if code == KeyCode::Backspace {
            select_completion(&mut app, "SELECT");
            let popup = app.active_console().completion.as_ref().unwrap();
            assert_eq!(
                popup.candidates[popup.selected].replace,
                TextRange::new(0, 6)
            );
        } else {
            assert!(app.active_console().completion.is_none());
        }
    }
}

#[test]
fn accepting_completion_without_text_change_ends_current_request() {
    let mut app = connected_completion_app();
    app.update(Action::ReplaceEditor(" ".into()));
    editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    app.update(Action::EditorPaste("SELECT".into()));
    app.update(Action::CompletionExplicit);
    select_completion(&mut app, "SELECT");
    assert!(app.active_console().completion_request.is_some());
    let revision = app.active_editor_revision();

    let commands = app.update(Action::CompletionAccept);

    assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
    assert_eq!(app.active_editor_revision(), revision);
    assert_completion_cursor(&app, 0, 7);
    assert!(app.active_console().completion.is_none());
    assert!(app.active_console().completion_request.is_none());
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::ScheduleCompletion(_)))
    );
}

#[test]
fn accepting_completion_ignores_pre_accept_due_event() {
    for word in ["sel", "SELECT"] {
        let mut app = connected_completion_app();
        app.update(Action::ReplaceEditor(" ".into()));
        editor_key(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        for character in word[..word.len() - 1].chars() {
            editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
        }
        let commands = app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char(word.chars().last().unwrap()),
            KeyModifiers::NONE,
        )));
        let key = commands
            .iter()
            .find_map(|command| match command {
                Command::ScheduleCompletion(key) => Some(key.clone()),
                _ => None,
            })
            .expect("actual input must schedule the pre-accept timer");
        assert_eq!(key.document_revision, app.active_editor_revision());
        assert_eq!(key.cursor, word.len());
        assert!(app.active_console().completion_request.is_some());
        app.update(Action::CompletionExplicit);
        select_completion(&mut app, "SELECT");
        app.update(Action::CompletionAccept);
        let revision = app.active_editor_revision();
        if word == "SELECT" {
            assert_eq!(key.document_revision, revision);
        }

        let commands = app.update(Action::CompletionDue(key));

        assert!(commands.is_empty());
        assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
        assert_eq!(app.active_editor_revision(), revision);
        assert_completion_cursor(&app, 0, 7);
        assert!(app.active_console().completion.is_none());
        assert!(app.active_console().completion_request.is_none());
    }
}

#[test]
fn typing_to_an_empty_result_closes_completion() {
    let mut app = App::new(Vec::new());
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    assert!(app.active_console().completion.is_some());

    editor_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);

    assert_eq!(app.active_editor_text().unwrap(), "selx");
    assert!(app.active_console().completion.is_none());
}

#[test]
fn paste_refreshes_an_open_completion_immediately() {
    let mut app = App::new(Vec::new());
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    assert!(app.active_console().completion.is_some());

    app.update(Action::EditorPaste("e".into()));

    assert_eq!(app.active_editor_text().unwrap(), "sele");
    assert!(app.active_console().completion.is_some());
    assert_eq!(
        app.active_console().completion.as_ref().unwrap().candidates[0].replace,
        lazydb::sql::TextRange::new(0, 4)
    );
}

#[test]
fn accepting_after_refresh_uses_the_latest_replace_range() {
    let mut app = App::new(Vec::new());
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    editor_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);

    app.update(Action::CompletionAccept);

    assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
    assert!(app.active_console().completion.is_none());
}

#[test]
fn refresh_preserves_the_selected_candidate_identity() {
    let mut app = App::new(Vec::new());
    for character in "select ".chars() {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    let initial = app.active_console().completion.as_ref().unwrap();
    assert!(
        initial.candidates.len() > 1,
        "expected multiple keyword candidates"
    );
    let selected = initial.candidates[1].clone();
    app.update(Action::CompletionNext);

    app.update(Action::EditorPaste(String::new()));

    let popup = app.active_console().completion.as_ref().unwrap();
    assert_eq!(popup.candidates[popup.selected].kind, selected.kind);
    assert_eq!(
        popup.candidates[popup.selected].insert_text,
        selected.insert_text
    );
}

#[test]
fn insert_ctrl_u_deletes_to_the_current_line_start() {
    let mut app = App::new(Vec::new());
    for character in "select 1\nfrom users".chars() {
        if character == '\n' {
            editor_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        } else {
            editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
        }
    }

    editor_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);

    assert_eq!(app.active_editor_text().unwrap(), "select 1\n");
}

#[tokio::test]
async fn sqlite_table_editor_applies_reviewed_unique_constraint() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use lazydb::{
        db::catalog::{CatalogId, CatalogKind, ObjectGroup},
        input::{keymap::Keymap, mouse::map_mouse},
        model::{catalog_editor::CatalogEditorPage, explorer::ExplorerNodeId},
        ui::{self, HitTarget, UiState},
    };
    use ratatui::{Terminal, backend::TestBackend};
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("constraints.db");
    let profile = import_connection_url(&format!("sqlite://{}", path.display()), None)
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let schema = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [profile.database.as_deref().unwrap(), "main"],
    );
    let mut app = App::new(vec![profile.clone()]);
    let (events, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![profile],
        HashSet::from([profile_id]),
        HashMap::new(),
        None,
        ProfileStore::new(temp.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        events,
    );
    let tables = ExplorerNodeId::Group {
        parent: schema,
        group: ObjectGroup::Tables,
    };
    app.update(Action::ExplorerSelect(ExplorerNodeId::Profile(profile_id)));
    dispatch(&mut app, &mut runtime, Action::ExplorerOpenSelected);
    wait_for_catalog_flow(&mut app, &mut runtime, &mut receiver, |app| {
        app.explorer
            .normalized
            .visible()
            .iter()
            .any(|node| node.id == tables)
    })
    .await;
    app.update(Action::ExplorerSelect(tables));
    app.update(Action::OpenCatalogCreate);
    assert_eq!(
        app.catalog_editor.as_ref().unwrap().page,
        CatalogEditorPage::Form
    );
    app.update(Action::CatalogEditorPaste("accounts".into()));
    app.update(Action::CatalogEditorOpenTableColumnDetails);
    app.update(Action::CatalogEditorPaste("username".into()));
    app.update(Action::CatalogEditorConfirmTableColumnDetails);

    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut ui = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut ui))
        .unwrap();
    let button = ui
        .hit_regions
        .iter()
        .find(|region| region.target == HitTarget::CatalogEditorAddTableConstraintUnique)
        .expect("Add UNIQUE is clickable at 80×20");
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: button.area.x + button.area.width / 2,
        row: button.area.y,
        modifiers: KeyModifiers::NONE,
    };
    let add_unique = map_mouse(click, &ui, &app).expect("click on Add UNIQUE");
    assert_eq!(add_unique, Action::CatalogEditorAddTableConstraintUnique);
    app.update(add_unique);
    let mut keymap = Keymap::default();
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let confirm = keymap.map(enter, &app).expect("confirm UNIQUE details");
    dispatch(&mut app, &mut runtime, confirm);
    dispatch(&mut app, &mut runtime, Action::CatalogEditorPreview);
    wait_for_catalog_flow(&mut app, &mut runtime, &mut receiver, |app| {
        app.catalog_editor
            .as_ref()
            .is_some_and(|editor| editor.page == CatalogEditorPage::SqlPreview)
    })
    .await;
    let plan = app.catalog_editor.as_ref().unwrap().plan.as_ref().unwrap();
    assert_eq!(plan.statements().len(), 1);
    assert!(plan.statements()[0].contains("UNIQUE (\"username\")"));

    let mut sqlite = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    let table_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = 'accounts'")
            .fetch_one(&mut sqlite)
            .await
            .unwrap();
    assert_eq!(table_count, 0);

    let apply = keymap.map(enter, &app).expect("apply reviewed SQL");
    dispatch(&mut app, &mut runtime, apply);
    wait_for_catalog_flow(&mut app, &mut runtime, &mut receiver, |app| {
        app.catalog_editor.is_none()
    })
    .await;
    let insert = "INSERT INTO accounts (username) VALUES ('ada')";
    sqlx::query(insert).execute(&mut sqlite).await.unwrap();
    let duplicate = sqlx::query(insert).execute(&mut sqlite).await.unwrap_err();
    assert!(
        duplicate
            .as_database_error()
            .is_some_and(|error| error.is_unique_violation()),
        "{duplicate}"
    );
    runtime.shutdown().await;
}

async fn wait_for_catalog_flow(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
    reached: impl Fn(&App) -> bool,
) {
    timeout(Duration::from_secs(5), async {
        while !reached(app) {
            let event = receiver.recv().await.expect("runtime event");
            match &event {
                Action::ConnectionFailed { message, .. }
                | Action::CatalogMutationPlanFailed { message, .. }
                | Action::CatalogMutationFailed { message, .. } => panic!("{message}"),
                _ => dispatch(app, runtime, event),
            }
        }
    })
    .await
    .expect("catalog editor did not reach the expected state");
}

#[tokio::test]
async fn connects_loads_catalog_and_executes_through_runtime() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("flow.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("flow")).unwrap();
    let profile = imported.profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::NewConsole);
    let (events, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![profile],
        HashSet::from([profile_id]),
        HashMap::new(),
        None,
        ProfileStore::new(temp.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        events,
    );

    dispatch(&mut app, &mut runtime, Action::RequestConnect(profile_id));
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(action, Action::ConnectionSucceeded { .. }));
    dispatch(&mut app, &mut runtime, action);
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    let tab_id = app.active_console().id;

    drain_runtime_events(&mut app, &mut runtime, &mut receiver).await;

    dispatch(
        &mut app,
        &mut runtime,
        Action::ReplaceEditor(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\n\
             INSERT INTO users VALUES (1, 'Ada');\n\
             SELECT id, name FROM users;"
                .into(),
        ),
    );
    dispatch(&mut app, &mut runtime, Action::RunAllSql);
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::ExecutionConfirm { .. })
    ));
    dispatch(
        &mut app,
        &mut runtime,
        Action::ToggleExecutionConfirmationFocus,
    );
    dispatch(&mut app, &mut runtime, Action::ConfirmExecution);
    timeout(Duration::from_secs(3), async {
        loop {
            let action = receiver.recv().await.unwrap();
            let terminal_query_action = matches!(
                &action,
                Action::QueryFinished {
                    tab_id: completed_tab_id,
                    ..
                }
                    | Action::QueryFailed {
                        tab_id: completed_tab_id,
                        ..
                    } if *completed_tab_id == tab_id
            );
            dispatch(&mut app, &mut runtime, action);
            if terminal_query_action {
                break;
            }
        }
    })
    .await
    .expect("SQL execution should finish within three seconds");

    let original_sql = app.active_editor_text().unwrap();
    let outcome = app.active_console().outcome.as_ref().unwrap_or_else(|| {
        panic!(
            "SQL execution should produce an outcome; output: {:?}",
            app.active_console().output
        )
    });
    assert_eq!(outcome.stats.row_count, 1);
    assert!(
        app.active_console()
            .output
            .iter()
            .any(|entry| entry.kind == lazydb::model::tab::OutputKind::Success)
    );
    assert_eq!(
        outcome.result_sets.last().unwrap().rows[0][1]
            .preview(20)
            .text,
        "Ada"
    );

    dispatch(&mut app, &mut runtime, Action::RefreshCatalog);
    drain_runtime_events(&mut app, &mut runtime, &mut receiver).await;
    assert!(app.explorer.nodes.iter().any(|node| node.name == "users"));

    let users = app.explorer.normalized.profiles[&profile_id]
        .catalog
        .entries()
        .iter()
        .find(|(_, entry)| entry.qualified_name.object == "users")
        .map(|(id, _)| id.clone())
        .unwrap();
    app.explorer.normalized.selected =
        Some(lazydb::model::explorer::ExplorerNodeId::Catalog(users));
    dispatch(&mut app, &mut runtime, Action::PreviewSelected);
    loop {
        let action = timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "relation preview timed out; tab state: {:?}",
                    app.tabs[app.active_tab]
                )
            })
            .unwrap();
        dispatch(&mut app, &mut runtime, action);
        if matches!(
            app.tabs[app.active_tab],
            lazydb::model::tab::WorkspaceTab::Relation(lazydb::model::relation::RelationTab {
                data: lazydb::model::relation::RelationLoad::Ready(_),
                ..
            })
        ) {
            break;
        }
    }
    drain_runtime_events(&mut app, &mut runtime, &mut receiver).await;
    assert!(matches!(
        app.tabs[app.active_tab],
        lazydb::model::tab::WorkspaceTab::Relation(lazydb::model::relation::RelationTab {
            data: lazydb::model::relation::RelationLoad::Ready(_),
            ..
        })
    ));

    dispatch(
        &mut app,
        &mut runtime,
        Action::SetRelationView(lazydb::model::relation::RelationView::Ddl),
    );
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert!(matches!(
        app.tabs[app.active_tab],
        lazydb::model::tab::WorkspaceTab::Relation(lazydb::model::relation::RelationTab {
            ddl: lazydb::model::relation::RelationLoad::Ready(_),
            ..
        })
    ));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.editor_text(tab_id).unwrap(), original_sql);

    app.update(Action::NewConsole);
    assert_eq!(app.tabs.len(), 3);
    runtime.shutdown().await;
}

fn dispatch(app: &mut App, runtime: &mut Runtime, action: Action) {
    for command in app.update(action) {
        runtime.dispatch(command);
    }
}

async fn drain_runtime_events(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) {
    loop {
        let Ok(Some(action)) = timeout(Duration::from_millis(100), receiver.recv()).await else {
            break;
        };
        dispatch(app, runtime, action);
    }
}

#[test]
fn ex_quit_is_reduced_by_app_not_called_directly() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("q".into()));
    let commands = app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(!app.should_quit);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, lazydb::action::Command::FlushWorkspace { .. }))
    );
}
