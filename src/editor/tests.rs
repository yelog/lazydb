use uuid::Uuid;

use crate::model::editor::{EditorMode, EditorPosition, EditorPromptKind, EditorViewport};

use super::{EditorKey, EditorWorkspace, decode_editor_text, encode_editor_text};

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
fn insert_control_keys_keep_vim_semantics() {
    let (mut workspace, id) = fixture("alpha beta");
    workspace.move_cursor_to_end(id).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "alpha ");
    workspace.press(id, EditorKey::Control('u')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "");
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

fn press_keys(workspace: &mut EditorWorkspace, id: Uuid, keys: &str) {
    for key in keys.chars() {
        workspace.press(id, EditorKey::Character(key)).unwrap();
    }
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
