use uuid::Uuid;

use crate::model::editor::{
    EditorHighlightKind, EditorMode, EditorPosition, EditorPromptKind, EditorViewport,
};

use super::{EditorEffect, EditorKey, EditorWorkspace, decode_editor_text, encode_editor_text};

#[test]
fn text_codec_preserves_exact_text() {
    for original in [
        "",
        "select 1",
        "select 1\n",
        "select 1\n\n",
        "数据",
        "🙂",
        "e\u{301}",
        "\tselect\t1",
    ] {
        let encoded = encode_editor_text(original);
        assert_eq!(decode_editor_text(&encoded).unwrap(), original);
    }
}

#[test]
fn text_codec_requires_the_sentinel_newline() {
    assert!(decode_editor_text("select 1").is_err());
}

fn fixture(text: &str) -> (EditorWorkspace, Uuid) {
    let id = Uuid::new_v4();
    let mut workspace = EditorWorkspace::new();
    workspace.open_console(id, text);
    (workspace, id)
}

fn insert_text(workspace: &mut EditorWorkspace, id: Uuid, text: &str) {
    for character in text.chars() {
        workspace
            .press(id, EditorKey::Character(character))
            .unwrap();
    }
}

#[test]
fn session_starts_insert_and_transitions_with_escape_and_i() {
    let (mut workspace, id) = fixture("");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Insert);
    workspace.press(id, EditorKey::Escape).unwrap();
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
    workspace.press(id, EditorKey::Character('i')).unwrap();
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Insert);
}

#[test]
fn editor_space_s_opens_manager_without_new_or_search_aliases() {
    let (mut workspace, id) = fixture("");
    workspace.press(id, EditorKey::Escape).unwrap();

    workspace.press(id, EditorKey::Character(' ')).unwrap();
    workspace.press(id, EditorKey::Character('s')).unwrap();
    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::OpenSqlEditorList]
    );

    for key in ['n', 'e'] {
        workspace.press(id, EditorKey::Character(' ')).unwrap();
        workspace.press(id, EditorKey::Character(key)).unwrap();
        assert!(workspace.drain_effects().is_empty());
    }
}

#[test]
fn unicode_positions_are_character_based() {
    let (mut workspace, id) = fixture("数据🙂");
    workspace.move_cursor_to_end(id).unwrap();
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 3 }
    );
    workspace.press(id, EditorKey::Backspace).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "数据");
    assert_eq!(workspace.position(id).unwrap().column, 2);
}

#[test]
fn multiline_paste_is_one_revision_and_undo_step() {
    let (mut workspace, id) = fixture("ab");
    workspace.move_cursor_to_end(id).unwrap();
    let revision = workspace.revision(id).unwrap();
    workspace.paste(id, "\n数据🙂").unwrap();
    assert_eq!(workspace.text(id).unwrap(), "ab\n数据🙂");
    assert_eq!(workspace.revision(id).unwrap(), revision + 1);
    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "ab");
}

#[test]
fn consecutive_insert_keys_undo_as_one_insert_session() {
    let (mut workspace, id) = fixture("");
    workspace.press(id, EditorKey::Character('a')).unwrap();
    workspace.press(id, EditorKey::Character('b')).unwrap();
    workspace.press(id, EditorKey::Character('c')).unwrap();
    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
}

#[test]
fn insert_session_undo_and_redo_restore_complete_text() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "select * from");
    workspace.press(id, EditorKey::Escape).unwrap();

    workspace.press(id, EditorKey::Character('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");

    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "select * from");
}

#[test]
fn platform_history_keys_in_insert_mode_preserve_insert_mode() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "hello");
    let modifier = if cfg!(target_os = "macos") {
        crossterm::event::KeyModifiers::SUPER
    } else {
        crossterm::event::KeyModifiers::CONTROL
    };
    let undo = crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('z'), modifier);
    let redo = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('z'),
        modifier | crossterm::event::KeyModifiers::SHIFT,
    );

    workspace.key(id, undo).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Insert);
    workspace.key(id, redo).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "hello");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Insert);
}

#[test]
fn non_platform_history_modifier_does_not_edit_sql_buffer() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "hello");
    let modifier = if cfg!(target_os = "macos") {
        crossterm::event::KeyModifiers::CONTROL
    } else {
        crossterm::event::KeyModifiers::SUPER
    };

    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('z'), modifier),
        )
        .unwrap();

    assert_eq!(workspace.text(id).unwrap(), "hello");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Insert);
}

#[test]
fn ctrl_history_in_prompt_does_not_edit_the_sql_buffer() {
    let (mut workspace, id) = normal_fixture("select 1");
    workspace.press(id, EditorKey::Character(':')).unwrap();
    press_keys(&mut workspace, id, "run");
    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('z'),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        )
        .unwrap();
    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('z'),
                crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::SHIFT,
            ),
        )
        .unwrap();
    assert_eq!(workspace.text(id).unwrap(), "select 1");
}

#[test]
fn undo_and_redo_restore_cursor_and_emit_one_change() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "数据🙂");
    workspace.move_cursor_to_end(id).unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();
    let final_position = workspace.position(id).unwrap();
    workspace.drain_effects();

    let revision = workspace.revision(id).unwrap();
    workspace.press(id, EditorKey::Character('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 0 }
    );
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
    assert_eq!(workspace.revision(id).unwrap(), revision + 1);
    assert!(matches!(
        workspace.drain_effects().as_slice(),
        [EditorEffect::Changed { .. }]
    ));

    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "数据🙂");
    assert_eq!(workspace.position(id).unwrap(), final_position);
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
    assert_eq!(workspace.revision(id).unwrap(), revision + 2);
    assert!(matches!(
        workspace.drain_effects().as_slice(),
        [EditorEffect::Changed { .. }]
    ));
}

#[test]
fn undo_and_redo_without_history_are_noops() {
    let (mut workspace, id) = normal_fixture("");
    let revision = workspace.revision(id).unwrap();

    workspace.press(id, EditorKey::Character('u')).unwrap();
    workspace.press(id, EditorKey::Control('r')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "");
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert!(workspace.drain_effects().is_empty());
}

#[test]
fn accepted_completion_stays_in_the_current_insert_transaction() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "select * from sysuser");
    workspace
        .replace_range(
            id,
            crate::sql::TextRange::new(14, 21),
            "sys_user",
            super::ReplacementCursor::EndOfInsertion,
        )
        .unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();

    workspace.press(id, EditorKey::Character('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");

    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "select * from sys_user");
}

#[test]
fn new_insert_after_undo_clears_redo() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "first");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('u')).unwrap();

    workspace.press(id, EditorKey::Character('i')).unwrap();
    insert_text(&mut workspace, id, "second");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('r')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "second");
}

#[test]
fn consecutive_insert_sessions_undo_and_redo_independently() {
    let (mut workspace, id) = fixture("");
    insert_text(&mut workspace, id, "one");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.move_cursor_to_end(id).unwrap();
    workspace.press(id, EditorKey::Character('i')).unwrap();
    insert_text(&mut workspace, id, " two");
    workspace.press(id, EditorKey::Escape).unwrap();

    workspace.press(id, EditorKey::Character('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "one");
    workspace.press(id, EditorKey::Character('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "one");
    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "one two");
}

#[test]
fn insert_control_keys_keep_vim_semantics() {
    let (mut workspace, id) = fixture("alpha beta");
    workspace.move_cursor_to_end(id).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "alpha ");
    workspace.press(id, EditorKey::Control('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
}

#[test]
fn normal_mode_ctrl_w_ctrl_w_emits_focus_next_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();

    assert_eq!(workspace.drain_effects(), vec![EditorEffect::FocusNext]);
}

#[test]
fn visual_mode_ctrl_w_ctrl_w_emits_focus_next_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('v')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();

    assert_eq!(workspace.drain_effects(), vec![EditorEffect::FocusNext]);
}

#[test]
fn normal_mode_counted_window_resize_emits_shared_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('5')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Character('>')).unwrap();

    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::ResizePane(
            crate::model::workspace::PaneResize {
                split: crate::model::workspace::PaneSplit::ExplorerWidth,
                delta: -5,
            }
        )]
    );
}

#[test]
fn normal_mode_window_reset_emits_reset_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Character('=')).unwrap();

    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::ResetPaneSizes]
    );
}

#[test]
fn normal_mode_window_f_emits_toggle_maximize_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Character('f')).unwrap();

    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::TogglePaneMaximized]
    );
}

#[test]
fn insert_control_u_deletes_only_to_the_current_unicode_line_start() {
    let (mut workspace, id) = fixture("select 1\n用户🙂");
    workspace.move_cursor_to_end(id).unwrap();
    let revision = workspace.revision(id).unwrap();

    workspace.press(id, EditorKey::Control('u')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "select 1\n");
    assert_eq!(workspace.revision(id).unwrap(), revision + 1);
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 1, column: 0 }
    );
    workspace.press(id, EditorKey::Control('u')).unwrap();
    assert_eq!(workspace.revision(id).unwrap(), revision + 1);
    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "select 1\n用户🙂");
}

#[test]
fn replacement_resets_cursor_mode_and_history() {
    let (mut workspace, id) = fixture("old");
    workspace.move_cursor_to_end(id).unwrap();
    workspace.set_text(id, "new").unwrap();
    assert_eq!(workspace.text(id).unwrap(), "new");
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 0 }
    );
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "new");
}

#[test]
fn replacement_cursor_controls_next_insert_position() {
    let (mut workspace, id) = fixture("SELE");
    workspace
        .replace_range(
            id,
            crate::sql::TextRange::new(0, 4),
            "SELECT",
            super::ReplacementCursor::EndOfInsertion,
        )
        .unwrap();
    assert_eq!(workspace.position(id).unwrap().column, 6);
    workspace.paste(id, " ").unwrap();
    assert_eq!(workspace.text(id).unwrap(), "SELECT ");
}

#[test]
fn named_and_system_registers_are_shared() {
    let (mut workspace, first) = fixture("");
    let second = Uuid::new_v4();
    workspace.open_console(second, "");
    workspace.set_register('a', "named");
    workspace.set_register('+', "clipboard");
    assert_eq!(workspace.register('a'), Some("named"));
    assert_eq!(workspace.register('*'), Some("clipboard"));
    assert_eq!(workspace.register('+'), Some("clipboard"));
    assert_eq!(workspace.text(first).unwrap(), "");
}

fn normal_fixture(text: &str) -> (EditorWorkspace, Uuid) {
    let (mut workspace, id) = fixture(text);
    workspace.press(id, EditorKey::Escape).unwrap();
    (workspace, id)
}

fn read_only_fixture(text: &str) -> (EditorWorkspace, Uuid) {
    let id = Uuid::new_v4();
    let mut workspace = EditorWorkspace::new();
    workspace.open_read_only(id, text);
    (workspace, id)
}

fn press_keys(workspace: &mut EditorWorkspace, id: Uuid, keys: &str) {
    for key in keys.chars() {
        workspace.press(id, EditorKey::Character(key)).unwrap();
    }
}

#[test]
fn current_scope_includes_visual_char_endpoint() {
    let (mut workspace, id) = normal_fixture("SELECT 1; SELECT 2;");
    press_keys(&mut workspace, id, "v7l");

    let scope = workspace
        .current_scope(id, crate::sql::SqlDialect::Generic)
        .unwrap()
        .unwrap();
    assert_eq!(scope.kind, crate::sql::ScopeKind::VisualChar);
    assert_eq!(scope.sql, "SELECT 1");
}

#[test]
fn current_scope_visual_line_uses_complete_selected_lines() {
    let (mut workspace, id) = normal_fixture("SELECT 1;\nSELECT 2;\nSELECT 3;");
    press_keys(&mut workspace, id, "Vj");

    let scope = workspace
        .current_scope(id, crate::sql::SqlDialect::Generic)
        .unwrap()
        .unwrap();
    assert_eq!(scope.kind, crate::sql::ScopeKind::VisualLine);
    assert_eq!(scope.sql, "SELECT 1;\nSELECT 2;");
}

#[test]
fn standalone_r_runs_current_sql_in_normal_and_visual_modes() {
    let (mut workspace, id) = normal_fixture("SELECT 1;");
    workspace.press(id, EditorKey::Character('R')).unwrap();
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunCurrent]);
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);

    press_keys(&mut workspace, id, "v6l");
    workspace.press(id, EditorKey::Character('R')).unwrap();
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunCurrent]);
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualChar);
}

#[test]
fn run_key_preserves_visual_shapes_and_does_not_replace_text() {
    for (selection, mode) in [
        ("V", EditorMode::VisualLine),
        ("\u{16}", EditorMode::VisualBlock),
    ] {
        let (mut workspace, id) = normal_fixture("SELECT 1;\nSELECT 2;");
        if selection == "\u{16}" {
            workspace.press(id, EditorKey::Control('v')).unwrap();
        } else {
            press_keys(&mut workspace, id, selection);
        }
        workspace.press(id, EditorKey::Character('R')).unwrap();

        assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunCurrent]);
        assert_eq!(workspace.mode(id).unwrap(), mode);
        assert_eq!(workspace.text(id).unwrap(), "SELECT 1;\nSELECT 2;");
    }
}

#[test]
fn run_key_does_not_intercept_insert_replace_or_operator_arguments() {
    let (mut workspace, id) = fixture("");
    press_keys(&mut workspace, id, "R");
    assert!(
        !workspace
            .drain_effects()
            .contains(&EditorEffect::RunCurrent)
    );
    assert_eq!(workspace.text(id).unwrap(), "R");

    let (mut workspace, id) = normal_fixture("abc");
    press_keys(&mut workspace, id, "rR");
    assert!(
        !workspace
            .drain_effects()
            .contains(&EditorEffect::RunCurrent)
    );
    assert_eq!(workspace.text(id).unwrap(), "Rbc");
}

#[test]
fn leader_run_keys_remain_distinct_for_current_and_full_buffer() {
    let (mut workspace, id) = normal_fixture("SELECT 1;");
    press_keys(&mut workspace, id, " r");
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunCurrent]);

    press_keys(&mut workspace, id, " R");
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunAll]);
}

#[test]
fn visual_leader_supports_scope_actions_without_leaking_normal_actions() {
    let (mut workspace, id) = normal_fixture("SELECT 1;");
    press_keys(&mut workspace, id, "v6l r");
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunCurrent]);
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualChar);

    press_keys(&mut workspace, id, " R");
    assert_eq!(workspace.drain_effects(), vec![EditorEffect::RunAll]);

    press_keys(&mut workspace, id, " q");
    assert!(workspace.drain_effects().is_empty());
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualChar);
}

#[test]
fn read_only_sessions_support_visual_yank_without_mutation() {
    let (mut workspace, id) = read_only_fixture("alpha beta\ngamma delta");
    let revision = workspace.revision(id).unwrap();

    press_keys(&mut workspace, id, "vey");

    assert_eq!(workspace.text(id).unwrap(), "alpha beta\ngamma delta");
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert_eq!(workspace.register('"'), Some("alpha"));
    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::Yanked("alpha".into())]
    );
}

#[test]
fn read_only_sessions_enter_visual_line_mode() {
    let (mut workspace, id) = read_only_fixture("alpha\nbeta");

    workspace.press(id, EditorKey::Character('V')).unwrap();

    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualLine);
    assert_eq!(workspace.text(id).unwrap(), "alpha\nbeta");
}

#[test]
fn normal_yank_emits_clipboard_effect() {
    let (mut workspace, id) = read_only_fixture("alpha beta");
    workspace.press(id, EditorKey::Character('y')).unwrap();
    workspace.press(id, EditorKey::Character('y')).unwrap();

    assert_eq!(workspace.register('"'), Some("alpha beta\n"));
    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::Yanked("alpha beta\n".into())]
    );
}

#[test]
fn read_only_sessions_ignore_editing_and_application_actions() {
    let (mut workspace, id) = read_only_fixture("alpha");
    let revision = workspace.revision(id).unwrap();

    press_keys(&mut workspace, id, "iX");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('Q')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "alpha");
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert!(workspace.drain_effects().is_empty());
}

#[test]
fn read_only_text_replacement_follows_tail_until_interaction() {
    let (mut workspace, id) = read_only_fixture("one");
    workspace.set_read_only_text(id, "one\ntwo", true).unwrap();
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 1, column: 3 }
    );

    workspace.press(id, EditorKey::Character('g')).unwrap();
    workspace
        .set_read_only_text(id, "one\ntwo\nthree", true)
        .unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 1);
}

#[test]
fn read_only_text_replacement_invalidates_cached_sql_highlights() {
    let (mut workspace, id) = read_only_fixture("");
    let viewport = EditorViewport {
        width: 120,
        height: 20,
    };
    let initial_revision = workspace.revision(id).unwrap();

    workspace
        .render_snapshot_with_dialect(id, viewport, crate::sql::SqlDialect::Postgres)
        .unwrap();
    workspace
        .set_read_only_text(id, "CREATE TABLE users (name TEXT DEFAULT 'Ada');", false)
        .unwrap();

    let snapshot = workspace
        .render_snapshot_with_dialect(id, viewport, crate::sql::SqlDialect::Postgres)
        .unwrap();
    assert_eq!(snapshot.revision, initial_revision + 1);
    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| { span.text == "CREATE" && span.kind == EditorHighlightKind::Keyword })
    );
    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| { span.text == "'Ada'" && span.kind == EditorHighlightKind::String })
    );

    workspace
        .set_read_only_text(id, "CREATE TABLE users (name TEXT DEFAULT 'Ada');", false)
        .unwrap();
    assert_eq!(workspace.revision(id).unwrap(), initial_revision + 1);
}

#[test]
fn render_snapshot_preserves_semantic_sql_highlight_kinds() {
    let (workspace, id) = fixture("SELECT u.id FROM users u");
    let snapshot = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 120,
                height: 20,
            },
            crate::sql::SqlDialect::Postgres,
        )
        .unwrap();
    let spans = &snapshot.lines[0].spans;

    assert!(
        spans
            .iter()
            .any(|span| { span.text == "users" && span.kind == EditorHighlightKind::Relation })
    );
    assert!(
        spans
            .iter()
            .any(|span| { span.text == "u" && span.kind == EditorHighlightKind::RelationAlias })
    );
    assert!(
        spans
            .iter()
            .any(|span| { span.text == "id" && span.kind == EditorHighlightKind::Column })
    );
}

#[test]
fn render_snapshot_preserves_semantic_kinds_before_incomplete_where() {
    let (workspace, id) = fixture("SELECT u.id FROM users u WHERE");
    let snapshot = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 120,
                height: 20,
            },
            crate::sql::SqlDialect::Postgres,
        )
        .unwrap();
    let spans = &snapshot.lines[0].spans;

    assert!(
        spans
            .iter()
            .any(|span| { span.text == "users" && span.kind == EditorHighlightKind::Relation })
    );
    assert!(
        spans
            .iter()
            .filter(|span| span.text == "u")
            .all(|span| span.kind == EditorHighlightKind::RelationAlias)
    );
    assert!(
        spans
            .iter()
            .any(|span| span.text == "id" && span.kind == EditorHighlightKind::Column)
    );
    assert!(
        spans
            .iter()
            .any(|span| span.text == "WHERE" && span.kind == EditorHighlightKind::Keyword)
    );
}

#[test]
fn vim_motions_are_table_driven_and_unicode_safe() {
    let cases = [
        ("3w", "one two three four", 0, 14),
        ("2b", "one two three four", 0, 0),
        ("0", "  one two", 0, 0),
        ("^", "  one two", 0, 2),
        ("$", "数据🙂", 0, 2),
        ("G", "one\ntwo\n数据🙂", 2, 0),
        ("}", "one\n\ntwo", 0, 0),
    ];

    for (keys, text, line, column) in cases {
        let (mut workspace, id) = normal_fixture(text);
        press_keys(&mut workspace, id, keys);
        assert_eq!(
            workspace.position(id).unwrap(),
            EditorPosition { line, column },
            "{keys}"
        );
    }
}

#[test]
fn vim_operators_and_text_objects_are_table_driven() {
    let cases = [
        ("dw", "one two", "two"),
        ("d2w", "one two three", "three"),
        ("dd", "one\ntwo", "two"),
        ("ciw", "one two", " two"),
        ("ci\"", "say \"hi\"", "say \"hi\""),
        ("da(", "say (hi)", "say (hi)"),
        ("~", "abc", "Abc"),
    ];

    for (keys, text, expected) in cases {
        let (mut workspace, id) = normal_fixture(text);
        press_keys(&mut workspace, id, keys);
        assert_eq!(workspace.text(id).unwrap(), expected, "{keys}");
    }
}

#[test]
fn visual_modes_track_char_line_and_block_shapes() {
    for (key, mode) in [
        ('v', EditorMode::VisualChar),
        ('V', EditorMode::VisualLine),
        ('\u{16}', EditorMode::VisualBlock),
    ] {
        let (mut workspace, id) = normal_fixture("one\ntwo");
        if key == '\u{16}' {
            workspace.press(id, EditorKey::Control('v')).unwrap();
        } else {
            workspace.press(id, EditorKey::Character(key)).unwrap();
        }
        assert_eq!(workspace.mode(id).unwrap(), mode);
        workspace.press(id, EditorKey::Escape).unwrap();
        assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
    }
}

#[test]
fn vim_undo_redo_and_dot_repeat_are_available() {
    let (mut workspace, id) = normal_fixture("one two");
    press_keys(&mut workspace, id, "dw");
    assert_eq!(workspace.text(id).unwrap(), "two");
    press_keys(&mut workspace, id, ".");
    assert_eq!(workspace.text(id).unwrap(), "");
    press_keys(&mut workspace, id, "u");
    assert_eq!(workspace.text(id).unwrap(), "two");
    workspace.press(id, EditorKey::Control('r')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
}

#[test]
fn unknown_actions_are_deferred_as_messages_and_q_is_quit() {
    let (mut workspace, id) = normal_fixture("");
    workspace.press(id, EditorKey::Character('Q')).unwrap();
    assert_eq!(workspace.drain_effects(), vec![super::EditorEffect::Quit]);

    workspace.press(id, EditorKey::Character('/')).unwrap();
    let snapshot = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 5,
            },
        )
        .unwrap();
    assert_eq!(
        snapshot.prompt.unwrap().kind,
        EditorPromptKind::SearchForward
    );
}

#[test]
fn prompt_search_supports_backward_repeat_and_abort() {
    let (mut workspace, id) = normal_fixture("one two one");
    workspace.press(id, EditorKey::Character('/')).unwrap();
    press_keys(&mut workspace, id, "one");
    workspace.press(id, EditorKey::Enter).unwrap();
    assert_eq!(workspace.position(id).unwrap().column, 8);
    workspace.press(id, EditorKey::Character('N')).unwrap();
    assert_eq!(workspace.position(id).unwrap().column, 0);
    workspace.press(id, EditorKey::Character('?')).unwrap();
    workspace.press(id, EditorKey::Character('t')).unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();
    assert!(
        workspace
            .render_snapshot(
                id,
                EditorViewport {
                    width: 80,
                    height: 5
                }
            )
            .unwrap()
            .prompt
            .is_none()
    );
}

#[test]
fn ex_commands_emit_shortcut_effects_and_invalid_commands_stay_editable() {
    let (mut workspace, id) = normal_fixture("");
    for (command, expected) in [
        ("run", super::EditorEffect::RunCurrent),
        ("runall", super::EditorEffect::RunAll),
        ("format", super::EditorEffect::FormatCurrent),
        (
            "tx auto",
            super::EditorEffect::SetTransactionModeRequested { manual: false },
        ),
        (
            "tx manual",
            super::EditorEffect::SetTransactionModeRequested { manual: true },
        ),
        ("tx clear", super::EditorEffect::ClearTransactionOutcome),
        ("commit", super::EditorEffect::Commit),
        ("rollback", super::EditorEffect::Rollback),
        ("q", super::EditorEffect::Quit),
    ] {
        workspace.press(id, EditorKey::Character(':')).unwrap();
        press_keys(&mut workspace, id, command);
        workspace.press(id, EditorKey::Enter).unwrap();
        assert_eq!(workspace.drain_effects(), vec![expected], ":{command}");
    }
    workspace.press(id, EditorKey::Character(':')).unwrap();
    press_keys(&mut workspace, id, "tx nope");
    workspace.press(id, EditorKey::Enter).unwrap();
    let prompt = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 5,
            },
        )
        .unwrap()
        .prompt
        .unwrap();
    assert_eq!(prompt.text, "tx nope");
    assert!(prompt.error.is_some());
    workspace.press(id, EditorKey::Escape).unwrap();
}

#[test]
fn ex_command_history_can_be_recalled_and_edited() {
    let (mut workspace, id) = normal_fixture("");
    workspace.press(id, EditorKey::Character(':')).unwrap();
    press_keys(&mut workspace, id, "run");
    workspace.press(id, EditorKey::Enter).unwrap();
    workspace.drain_effects();
    workspace.press(id, EditorKey::Character(':')).unwrap();
    workspace.press(id, EditorKey::Up).unwrap();
    press_keys(&mut workspace, id, "all");
    let prompt = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 5,
            },
        )
        .unwrap()
        .prompt
        .unwrap();
    assert_eq!(prompt.text, "runall");
}

#[test]
fn prompt_history_preserves_raw_text_and_display_projection_is_inert() {
    let (mut workspace, id) = normal_fixture("");
    workspace.press(id, EditorKey::Character(':')).unwrap();
    workspace.paste(id, "run\x1b[31m").unwrap();
    let snapshot = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 5,
            },
        )
        .unwrap();
    assert_eq!(snapshot.prompt.unwrap().text, "run<ESC>[31m");
    workspace.press(id, EditorKey::Escape).unwrap();
}

fn ex(workspace: &mut EditorWorkspace, id: Uuid, command: &str) {
    workspace.press(id, EditorKey::Character(':')).unwrap();
    workspace.paste(id, command).unwrap();
    workspace.press(id, EditorKey::Enter).unwrap();
}

#[test]
fn substitute_current_line_supports_ampersand_and_capture_groups() {
    let (mut workspace, id) = normal_fixture("one one\ntwo");
    ex(&mut workspace, id, "s/(one)/<$1>&/");
    assert_eq!(workspace.text(id).unwrap(), "<one>one one\ntwo");
    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "one one\ntwo");
}

#[test]
fn substitute_percent_global_case_and_custom_delimiter() {
    let (mut workspace, id) = normal_fixture("Foo foo\nfoo");
    ex(&mut workspace, id, "%s#foo#bar#gi");
    assert_eq!(workspace.text(id).unwrap(), "bar bar\nbar");
}

#[test]
fn substitute_numeric_range_only_changes_selected_lines() {
    let (mut workspace, id) = normal_fixture("one\none\none");
    ex(&mut workspace, id, "1,2s/one/X/g");
    assert_eq!(workspace.text(id).unwrap(), "X\nX\none");
}

#[test]
fn substitute_reuses_previous_pattern_and_reports_errors() {
    let (mut workspace, id) = normal_fixture("one two\none");
    ex(&mut workspace, id, "s/one/ONE/");
    ex(&mut workspace, id, "%s//X/");
    assert_eq!(workspace.text(id).unwrap(), "ONE two\nX");
    ex(&mut workspace, id, "s/[broken/X/");
    let prompt = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 5,
            },
        )
        .unwrap()
        .prompt
        .unwrap();
    assert!(prompt.error.unwrap().contains("invalid regex"));
}

#[test]
fn substitute_confirmation_is_immutable_and_cancelable() {
    let (mut workspace, id) = normal_fixture("one one");
    ex(&mut workspace, id, "s/one/two/gc");
    assert_eq!(
        workspace.drain_effects(),
        vec![super::EditorEffect::SubstituteConfirmRequested { count: 2 }]
    );
    workspace.substitute_confirm(true, false, false).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "one one");
    workspace.cancel_substitute();
    assert_eq!(workspace.text(id).unwrap(), "one one");
}
