use uuid::Uuid;

use crate::model::editor::{
    EditorHighlightKind, EditorMode, EditorPosition, EditorPromptKind, EditorSelectionShape,
    EditorViewport,
};
use crate::model::editor_language::EditorLanguage;

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

#[test]
fn vertical_drag_scroll_clamps_at_both_ends_and_preserves_horizontal_offset() {
    let (mut workspace, id) = fixture(&"wide-column-界\n".repeat(100));
    let viewport = EditorViewport {
        width: 5,
        height: 4,
    };
    workspace.set_viewport(id, viewport).unwrap();
    workspace.scroll(id, 0, 3).unwrap();
    assert!(!workspace.can_scroll_vertically(id, -1));
    assert!(workspace.can_scroll_vertically(id, 1));
    workspace.scroll(id, isize::MAX, 0).unwrap();
    let snapshot = workspace.render_plain_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 97);
    assert_eq!(snapshot.horizontal_offset, 3);
    assert!(!workspace.can_scroll_vertically(id, 1));
    assert!(workspace.can_scroll_vertically(id, -1));
    workspace.scroll(id, isize::MIN, 0).unwrap();
    assert!(!workspace.can_scroll_vertically(id, -1));
    assert_eq!(
        workspace
            .render_plain_snapshot(id, viewport)
            .unwrap()
            .first_line,
        0
    );
}

#[test]
fn wrapped_drag_scroll_reaches_the_final_source_line() {
    let text = (1..=120)
        .map(|line| format!("REDIS_LINE_{line:03}: {}", "synthetic text ".repeat(4)))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = fixture(&text);
    let viewport = EditorViewport {
        width: 46,
        height: 33,
    };
    workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();
    workspace.scroll(id, isize::MAX, 0).unwrap();
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();
    assert_eq!(snapshot.lines.last().unwrap().line, 119);
    assert!(
        snapshot
            .lines
            .iter()
            .any(|line| line.display_text.contains("REDIS_LINE_120"))
    );
    assert!(!workspace.can_scroll_vertically(id, 1));
}

#[test]
fn short_and_wrapped_previews_stop_drag_scrolling_at_their_rendered_limits() {
    let (mut workspace, id) = fixture("short");
    let viewport = EditorViewport {
        width: 5,
        height: 4,
    };
    workspace.set_viewport(id, viewport).unwrap();
    assert!(!workspace.can_scroll_vertically(id, 1));
    assert!(!workspace.can_scroll_vertically(id, -1));
    let (mut workspace, id) = fixture(&"word ".repeat(20));
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();
    assert!(snapshot.total_lines > viewport.height);
    assert!(workspace.can_scroll_vertically(id, 1));
    workspace.scroll(id, 99, 0).unwrap();
    assert!(!workspace.can_scroll_vertically(id, 1));
    assert!(workspace.can_scroll_vertically(id, -1));
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();
    assert_eq!(snapshot.first_line, snapshot.total_lines - viewport.height);
}

#[test]
fn redis_value_session_supports_vim_editing_search_yank_paste_undo_redo_and_save() {
    let id = Uuid::new_v4();
    let mut workspace = EditorWorkspace::new();
    workspace.open_value(id, "one\ntwo");

    workspace.press(id, EditorKey::Character('i')).unwrap();
    workspace.press(id, EditorKey::Character('X')).unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();
    assert!(workspace.text(id).unwrap().contains('X'));

    workspace.press(id, EditorKey::Character('/')).unwrap();
    insert_text(&mut workspace, id, "two");
    workspace.press(id, EditorKey::Enter).unwrap();
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);

    workspace.press(id, EditorKey::Character('y')).unwrap();
    workspace.press(id, EditorKey::Character('y')).unwrap();
    workspace.press(id, EditorKey::Character('p')).unwrap();
    workspace.undo(id).unwrap();
    workspace.redo(id).unwrap();
    workspace.press(id, EditorKey::Control('s')).unwrap();
    assert!(workspace
        .drain_effects()
        .iter()
        .any(|effect| matches!(effect, EditorEffect::SaveRequested { console_id, .. } if *console_id == id)));
}

#[test]
fn redis_value_replace_digit_is_immediate() {
    let id = Uuid::new_v4();
    let mut workspace = EditorWorkspace::new();
    workspace.open_value(id, "abc\ndef");
    workspace.press(id, EditorKey::Escape).unwrap();
    let revision = workspace.revision(id).unwrap();
    workspace.drain_effects();

    workspace.press(id, EditorKey::Character('r')).unwrap();
    workspace.press(id, EditorKey::Character('2')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "2bc\ndef");
    assert!(workspace.revision(id).unwrap() > revision);
    assert!(workspace.drain_effects().iter().any(|effect| {
        matches!(effect, EditorEffect::Changed { console_id, .. } if *console_id == id)
    }));

    let revision = workspace.revision(id).unwrap();
    workspace.press(id, EditorKey::Character('j')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "2bc\ndef");
    assert_eq!(workspace.revision(id).unwrap(), revision);
}

#[test]
fn normal_mode_character_arguments_are_not_counts() {
    for (command, argument, expected) in [('r', '2', "2bc"), ('f', 'b', "abc"), ('t', 'b', "abc")] {
        let id = Uuid::new_v4();
        let mut workspace = EditorWorkspace::new();
        workspace.open_value(id, "abc\ndef");
        workspace.press(id, EditorKey::Escape).unwrap();
        workspace.press(id, EditorKey::Character(command)).unwrap();
        workspace.press(id, EditorKey::Character(argument)).unwrap();
        assert!(workspace.text(id).unwrap().starts_with(expected));
    }
}

#[test]
fn cancelled_character_argument_does_not_capture_the_next_key() {
    let (mut workspace, id) = normal_fixture("abc");
    workspace.press(id, EditorKey::Character('r')).unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('2')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "abc");
}

#[test]
fn redis_value_wrapped_preview_refreshes_after_undo() {
    let id = Uuid::new_v4();
    let mut workspace = EditorWorkspace::new();
    workspace.open_value(
        id,
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten",
    );
    let viewport = EditorViewport {
        width: 80,
        height: 24,
    };

    workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();
    workspace.press(id, EditorKey::Character('G')).unwrap();
    workspace.press(id, EditorKey::Character('o')).unwrap();
    workspace.press(id, EditorKey::Enter).unwrap();
    workspace.press(id, EditorKey::Character('x')).unwrap();
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();

    workspace.press(id, EditorKey::Character('u')).unwrap();
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, EditorLanguage::Plain, true)
        .unwrap();

    assert_eq!(
        workspace.text(id).unwrap(),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten"
    );
    assert_eq!(snapshot.logical_line_count, 10);
    assert!(snapshot.lines.iter().all(|line| line.line < 10));
}

fn insert_text(workspace: &mut EditorWorkspace, id: Uuid, text: &str) {
    for character in text.chars() {
        workspace
            .press(id, EditorKey::Character(character))
            .unwrap();
    }
}

#[test]
fn rendering_replaces_old_revision_highlights() {
    let (mut workspace, id) = fixture("select 1");
    let viewport = EditorViewport {
        width: 80,
        height: 10,
    };

    workspace
        .render_snapshot_with_dialect(id, viewport, crate::sql::SqlDialect::Postgres)
        .unwrap();
    assert_eq!(workspace.analysis_cache.borrow().len(), 1);

    for character in [' ', 'f', 'r', 'o', 'm', ' ', 't', 'a', 'b', 'l', 'e'] {
        workspace
            .press(id, EditorKey::Character(character))
            .unwrap();
        workspace
            .render_snapshot_with_dialect(id, viewport, crate::sql::SqlDialect::Postgres)
            .unwrap();
    }
    assert_eq!(workspace.analysis_cache.borrow().len(), 1);
}

#[test]
fn preview_rendering_does_not_populate_sql_analysis_cache() {
    let (workspace, id) = fixture(r#"{"message":"数据","ok":true}"#);
    let snapshot = workspace
        .render_preview_snapshot(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::model::editor_language::EditorLanguage::Json,
        )
        .unwrap();

    assert!(workspace.analysis_cache.borrow().is_empty());
    assert!(
        snapshot
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.kind == EditorHighlightKind::String)
    );
}

#[test]
fn ddl_preview_and_sql_editor_share_highlight_kinds() {
    let text = r#"CREATE TABLE users (id bigint, name text);
SELECT id, name FROM users;"#;
    let (mut workspace, ddl_id) = fixture(text);
    let editor_id = Uuid::new_v4();
    workspace.open_console(editor_id, text);
    let viewport = EditorViewport {
        width: 120,
        height: 10,
    };
    let ddl = workspace
        .render_snapshot_with_dialect_and_statement(
            ddl_id,
            viewport,
            crate::sql::SqlDialect::Postgres,
            None,
        )
        .unwrap();
    let editor = workspace
        .render_snapshot_with_dialect_and_statement(
            editor_id,
            viewport,
            crate::sql::SqlDialect::Postgres,
            Some(crate::sql::TextRange::new(0, text.len())),
        )
        .unwrap();
    let ddl_spans = ddl
        .lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| (&span.text, span.kind)))
        .collect::<Vec<_>>();
    let editor_spans = editor
        .lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| (&span.text, span.kind)))
        .collect::<Vec<_>>();
    assert_eq!(ddl_spans, editor_spans);
}

#[test]
fn closing_console_releases_its_highlight_cache() {
    let (mut workspace, id) = fixture("select 1");
    let other_id = Uuid::new_v4();
    workspace.open_console(other_id, "select 2");
    let viewport = EditorViewport {
        width: 80,
        height: 10,
    };

    workspace
        .render_snapshot_with_dialect(id, viewport, crate::sql::SqlDialect::Postgres)
        .unwrap();
    workspace
        .render_snapshot_with_dialect(other_id, viewport, crate::sql::SqlDialect::Postgres)
        .unwrap();
    assert_eq!(workspace.analysis_cache.borrow().len(), 2);

    workspace.close_console(id);
    assert_eq!(workspace.analysis_cache.borrow().len(), 1);
    assert!(
        workspace
            .analysis_cache
            .borrow()
            .keys()
            .all(|key| key.console_id == other_id)
    );
}

#[test]
fn line_count_does_not_require_a_render_snapshot() {
    let (workspace, id) = fixture("select 1\nselect 2");

    assert_eq!(workspace.line_count(id).unwrap(), 2);
    assert!(workspace.analysis_cache.borrow().is_empty());
}

#[test]
fn cursor_navigation_keeps_the_cursor_inside_the_viewport() {
    let text = (0..12)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    workspace
        .set_viewport(
            id,
            EditorViewport {
                width: 12,
                height: 3,
            },
        )
        .unwrap();

    for _ in 0..10 {
        workspace.press(id, EditorKey::Character('j')).unwrap();
    }

    let snapshot = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 12,
                height: 3,
            },
        )
        .unwrap();
    assert_eq!(snapshot.cursor.line, 10);
    assert!(snapshot.cursor_screen_cell.is_some());
    assert!(snapshot.first_line > 0);
    assert!(snapshot.first_line + snapshot.viewport.height > snapshot.cursor.line);
}

#[test]
fn offscreen_search_repositions_the_viewport_and_horizontal_cursor() {
    let long_line = format!("{}needle", "x".repeat(80));
    let text = (0..8)
        .map(|line| format!("line-{line}"))
        .chain([long_line])
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    let viewport = EditorViewport {
        width: 10,
        height: 3,
    };
    workspace.set_viewport(id, viewport).unwrap();
    workspace.press(id, EditorKey::Character('/')).unwrap();
    press_keys(&mut workspace, id, "needle");
    workspace.press(id, EditorKey::Enter).unwrap();

    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.cursor.line, 8);
    assert!(snapshot.cursor_screen_cell.is_some());
    assert!(snapshot.horizontal_offset > 0);
}

#[test]
fn snapshot_does_not_place_cursor_in_overscan_rows() {
    let (mut workspace, id) = read_only_fixture("one\ntwo\nthree");
    workspace
        .set_viewport(
            id,
            EditorViewport {
                width: 20,
                height: 1,
            },
        )
        .unwrap();
    workspace.move_cursor_to_end(id).unwrap();

    let snapshot = workspace
        .render_snapshot(
            id,
            EditorViewport {
                width: 20,
                height: 1,
            },
        )
        .unwrap();
    assert!(snapshot.cursor_screen_cell.is_none());
}

#[test]
fn normal_mode_page_keys_scroll_the_editor_without_editing_text() {
    let text = (0..40)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    let viewport = EditorViewport {
        width: 20,
        height: 5,
    };
    workspace.set_viewport(id, viewport).unwrap();
    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('f'),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        )
        .unwrap();
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert!(snapshot.first_line > 0);
    let paged_position = workspace.position(id).unwrap();
    assert!(paged_position.line >= snapshot.first_line);
    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('j'),
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .unwrap();
    let after_move = workspace.render_snapshot(id, viewport).unwrap();
    assert!(after_move.first_line >= snapshot.first_line);
    assert!(workspace.position(id).unwrap().line > paged_position.line);
    assert_eq!(workspace.text(id).unwrap(), text);

    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('b'),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        )
        .unwrap();
    assert_eq!(
        workspace.render_snapshot(id, viewport).unwrap().first_line,
        0
    );
}

#[test]
fn normal_mode_half_page_keys_keep_cursor_visible_without_editing_text() {
    let text = (0..40)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    let viewport = EditorViewport {
        width: 20,
        height: 5,
    };
    workspace.set_viewport(id, viewport).unwrap();
    let original = workspace.text(id).unwrap();
    let revision = workspace.revision(id).unwrap();

    for key in [EditorKey::Control('d'), EditorKey::Control('u')] {
        workspace.press(id, key).unwrap();
        let snapshot = workspace.render_snapshot(id, viewport).unwrap();
        assert!(snapshot.cursor_screen_cell.is_some(), "key={key:?}");
        assert!(snapshot.cursor.line >= snapshot.first_line);
        assert!(snapshot.cursor.line < snapshot.first_line + viewport.height);
    }

    assert_eq!(workspace.text(id).unwrap(), original);
    assert_eq!(workspace.revision(id).unwrap(), revision);
}

#[test]
fn navigation_keeps_unicode_and_tab_cursor_visible_at_horizontal_edges() {
    let text = "short\n\t数据数据数据数据数据数据数据数据\nend";
    let (mut workspace, id) = read_only_fixture(text);
    let viewport = EditorViewport {
        width: 8,
        height: 2,
    };
    workspace.set_viewport(id, viewport).unwrap();

    for _ in 0..2 {
        workspace.press(id, EditorKey::Character('j')).unwrap();
    }
    for _ in 0..20 {
        workspace.press(id, EditorKey::Character('l')).unwrap();
    }
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert!(snapshot.cursor_screen_cell.is_some());

    workspace.press(id, EditorKey::Character('h')).unwrap();
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert!(snapshot.cursor_screen_cell.is_some());
}

#[test]
fn forward_search_starts_after_a_unicode_cursor_boundary() {
    let (mut workspace, id) = read_only_fixture("数据 数据");
    workspace.move_cursor_to_end(id).unwrap();
    workspace.press(id, EditorKey::Character('g')).unwrap();
    workspace.press(id, EditorKey::Character('g')).unwrap();
    workspace.press(id, EditorKey::Character('/')).unwrap();
    press_keys(&mut workspace, id, "数据");
    workspace.press(id, EditorKey::Enter).unwrap();
    assert_eq!(workspace.position(id).unwrap().column, 3);
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
fn editor_space_m_opens_notification_history_in_normal_mode() {
    let (mut workspace, id) = fixture("SELECT 1;");
    workspace.press(id, EditorKey::Escape).unwrap();

    workspace.press(id, EditorKey::Character(' ')).unwrap();
    workspace.press(id, EditorKey::Character('m')).unwrap();

    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::OpenNotificationHistory]
    );
    assert_eq!(workspace.text(id).unwrap(), "SELECT 1;");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);
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
fn mouse_range_returns_exact_source_text_without_creating_selection() {
    let (workspace, id) = fixture("SELECT 数据\n🙂 FROM users");

    let selected = workspace
        .mouse_range_text(
            id,
            EditorPosition { line: 1, column: 5 },
            EditorPosition { line: 0, column: 7 },
        )
        .unwrap();

    assert_eq!(selected, "数据\n🙂 FROM");
    assert_eq!(workspace.mouse_selection(id).unwrap(), None);
}

#[test]
fn mouse_selection_preserves_revision_history_and_read_only_capability() {
    let (mut workspace, id) = read_only_fixture("alpha beta");
    let revision = workspace.revision(id).unwrap();

    assert_eq!(
        workspace
            .mouse_range_text(
                id,
                EditorPosition { line: 0, column: 0 },
                EditorPosition { line: 0, column: 4 },
            )
            .unwrap(),
        "alpha"
    );
    assert_eq!(workspace.text(id).unwrap(), "alpha beta");
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert!(workspace.drain_effects().is_empty());

    workspace.undo(id).unwrap();
    assert_eq!(workspace.text(id).unwrap(), "alpha beta");
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert!(workspace.drain_effects().is_empty());
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
fn completion_cursor_uses_the_target_line_length() {
    for (text, range, replacement, expected) in [
        (
            "sel\n",
            crate::sql::TextRange::new(0, 3),
            "SELECT",
            EditorPosition { line: 0, column: 6 },
        ),
        (
            "sel\nx",
            crate::sql::TextRange::new(0, 3),
            "SELECT",
            EditorPosition { line: 0, column: 6 },
        ),
        (
            "-- heading\nsel\n",
            crate::sql::TextRange::new(11, 14),
            "SELECT",
            EditorPosition { line: 1, column: 6 },
        ),
    ] {
        let (mut workspace, id) = fixture(text);
        workspace
            .replace_range(
                id,
                range,
                replacement,
                super::ReplacementCursor::EndOfInsertion,
            )
            .unwrap();
        assert_eq!(workspace.position(id).unwrap(), expected);
    }
}

#[test]
fn completion_noop_replacement_moves_cursor_without_revision() {
    let (mut workspace, id) = fixture("SELECT");
    workspace.set_mode(id, EditorMode::Insert).unwrap();
    workspace
        .replace_range(
            id,
            crate::sql::TextRange::new(0, 6),
            "SELECT",
            super::ReplacementCursor::EndOfInsertion,
        )
        .unwrap();

    assert_eq!(workspace.text(id).unwrap(), "SELECT");
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 6 }
    );
    assert_eq!(workspace.revision(id).unwrap(), 0);
    assert!(workspace.drain_effects().is_empty());
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
fn normal_jump_list_round_trips_through_large_motions() {
    let (mut workspace, id) = normal_fixture("first\nsecond\nthird");
    workspace.press(id, EditorKey::Character('G')).unwrap();
    workspace.press(id, EditorKey::Character('g')).unwrap();
    workspace.press(id, EditorKey::Character('g')).unwrap();

    workspace.press(id, EditorKey::Control('o')).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 2);
    workspace.press(id, EditorKey::Control('o')).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 0);
    workspace.press(id, EditorKey::Control('i')).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 2);
}

#[test]
fn jump_navigation_does_not_change_text_or_revision() {
    let (mut workspace, id) = normal_fixture("first\nsecond");
    let text = workspace.text(id).unwrap();
    let revision = workspace.revision(id).unwrap();
    workspace.press(id, EditorKey::Character('G')).unwrap();
    workspace.press(id, EditorKey::Control('o')).unwrap();
    assert_eq!(workspace.text(id).unwrap(), text);
    assert_eq!(workspace.revision(id).unwrap(), revision);
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
fn visual_char_selection_cells_are_direction_independent() {
    let viewport = EditorViewport {
        width: 80,
        height: 10,
    };

    let (mut forward, forward_id) = normal_fixture("abcdef");
    press_keys(&mut forward, forward_id, "vll");
    let forward_snapshot = forward
        .render_snapshot_with_dialect(forward_id, viewport, crate::sql::SqlDialect::Generic)
        .unwrap();

    let (mut reverse, reverse_id) = normal_fixture("abcdef");
    press_keys(&mut reverse, reverse_id, "llvhh");
    let reverse_snapshot = reverse
        .render_snapshot_with_dialect(reverse_id, viewport, crate::sql::SqlDialect::Generic)
        .unwrap();

    assert_eq!(forward_snapshot.selection_cells, vec![(0, 0, 3)]);
    assert_eq!(
        reverse_snapshot.selection_cells,
        forward_snapshot.selection_cells
    );
}

#[test]
fn visual_char_selection_marks_only_crossed_newlines() {
    let (mut workspace, id) = normal_fixture("first\nsecond\nthird");
    press_keys(&mut workspace, id, "vj");

    let snapshot = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Generic,
        )
        .unwrap();

    assert_eq!(
        snapshot
            .lines
            .iter()
            .map(|line| line.selection_newline)
            .collect::<Vec<_>>(),
        vec![true, false, false]
    );
}

#[test]
fn visual_char_selection_cells_use_display_width_for_unicode_text() {
    let (mut workspace, id) = normal_fixture("界🙂ab");
    press_keys(&mut workspace, id, "vll");

    let snapshot = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Generic,
        )
        .unwrap();

    assert_eq!(snapshot.selection_cells, vec![(0, 0, 5)]);
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
fn read_only_multiline_sql_yank_preserves_whitespace() {
    let text = "SELECT\n\t'a  b';";
    let (mut workspace, id) = read_only_fixture(text);
    let revision = workspace.revision(id).unwrap();

    press_keys(&mut workspace, id, "ggVGy");

    assert_eq!(workspace.text(id).unwrap(), text);
    assert_eq!(workspace.revision(id).unwrap(), revision);
    assert_eq!(workspace.register('"'), Some("SELECT\n\t'a  b';\n"));
    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::Yanked("SELECT\n\t'a  b';\n".into())]
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
fn output_tail_text_update_repositions_after_interaction() {
    let (mut workspace, id) = read_only_fixture("one");
    workspace.set_read_only_text(id, "one\ntwo", true).unwrap();
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 1, column: 0 }
    );

    press_keys(&mut workspace, id, "gg");
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 0 }
    );

    workspace
        .set_read_only_text(id, "one\ntwo\nthree", true)
        .unwrap();
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 2, column: 0 }
    );
}

#[test]
fn output_tail_snapshot_shows_last_block_without_prior_viewport() {
    let text = (0..10)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    workspace
        .set_read_only_text(id, &format!("{text}\ntail-marker"), true)
        .unwrap();

    let viewport = EditorViewport {
        width: 40,
        height: 3,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 8);
    assert_eq!(snapshot.horizontal_offset, 0);
    assert!(snapshot.lines.iter().any(|line| {
        line.line == 10
            && line
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
                == "tail-marker"
    }));
}

#[test]
fn output_tail_short_content_stays_at_first_line() {
    let (mut workspace, id) = read_only_fixture("one");
    workspace.set_read_only_text(id, "one\ntwo", true).unwrap();
    let viewport = EditorViewport {
        width: 40,
        height: 3,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 0);
    assert_eq!(snapshot.total_lines, 2);
}

#[test]
fn output_tail_trailing_newline_keeps_empty_logical_line() {
    let (mut workspace, id) = read_only_fixture("one");
    workspace
        .set_read_only_text(id, "one\ntwo\n", true)
        .unwrap();
    let viewport = EditorViewport {
        width: 40,
        height: 3,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 0);
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 2, column: 0 }
    );
}

#[test]
fn output_tail_long_last_line_keeps_horizontal_offset_zero() {
    let long = "x".repeat(120);
    let (mut workspace, id) = read_only_fixture("head");
    workspace
        .set_read_only_text(id, &format!("head\n{long}"), true)
        .unwrap();
    let viewport = EditorViewport {
        width: 10,
        height: 3,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 0);
    assert_eq!(snapshot.horizontal_offset, 0);
    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 1, column: 0 }
    );
}

#[test]
fn output_tail_viewport_sync_matches_snapshot_and_respects_manual_scroll() {
    let text = (0..10)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut workspace, id) = read_only_fixture(&text);
    workspace
        .set_read_only_text(id, &format!("{text}\ntail-marker"), true)
        .unwrap();

    let viewport = EditorViewport {
        width: 40,
        height: 3,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 8);
    workspace.sync_output_viewport(id, viewport).unwrap();
    let after_sync = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(after_sync.first_line, 8);

    workspace.scroll(id, -3, 0).unwrap();
    let after_scroll = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(after_scroll.first_line, 5);
    workspace.sync_output_viewport(id, viewport).unwrap();
    let after_idle_sync = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(after_idle_sync.first_line, 5);

    workspace
        .set_read_only_text(id, &format!("{text}\ntail-marker\nnew-tail"), true)
        .unwrap();
    let tail_snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(tail_snapshot.first_line, 9);
    workspace.sync_output_viewport(id, viewport).unwrap();
    let tail_after = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(tail_after.first_line, 9);
    assert!(tail_after.lines.iter().any(|line| {
        line.line == 11
            && line
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
                == "new-tail"
    }));
}

#[test]
fn setting_one_scroll_axis_preserves_the_other_axis() {
    let (mut workspace, id) = read_only_fixture(
        "01234567890123456789\n01234567890123456789\n01234567890123456789\n01234567890123456789\n01234567890123456789",
    );
    let viewport = EditorViewport {
        width: 5,
        height: 2,
    };
    workspace.sync_output_viewport(id, viewport).unwrap();
    workspace.set_scroll_offset(id, 2, 7).unwrap();
    workspace.set_scroll_axis(id, true, 1).unwrap();
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 1);
    assert_eq!(snapshot.horizontal_offset, 7);

    workspace.set_scroll_axis(id, false, 3).unwrap();
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 1);
    assert_eq!(snapshot.horizontal_offset, 3);
}

#[test]
fn output_tail_request_survives_zero_size_viewport_sync() {
    let (mut workspace, id) = read_only_fixture("one");
    workspace
        .set_read_only_text(id, "one\ntwo\nthree\nfour\nfive", true)
        .unwrap();
    workspace
        .sync_output_viewport(
            id,
            EditorViewport {
                width: 0,
                height: 0,
            },
        )
        .unwrap();

    let viewport = EditorViewport {
        width: 40,
        height: 2,
    };
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 3);
    workspace.sync_output_viewport(id, viewport).unwrap();
    let after = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(after.first_line, 3);
}

#[test]
fn output_tail_false_update_cancels_pending_request() {
    let (mut workspace, id) = read_only_fixture("one");
    let viewport = EditorViewport {
        width: 40,
        height: 3,
    };
    workspace.set_read_only_text(id, "one\ntwo", true).unwrap();
    workspace.set_read_only_text(id, "three", false).unwrap();
    let snapshot = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(snapshot.first_line, 0);
    workspace.sync_output_viewport(id, viewport).unwrap();
    let after = workspace.render_snapshot(id, viewport).unwrap();
    assert_eq!(after.first_line, 0);
    assert_eq!(snapshot.total_lines, 1);
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
fn render_snapshot_uses_full_document_source_ranges_for_later_lines() {
    let text = "SELECT * from ignore_table;\n\nSELECT ignore_col from sys_user;";
    let (workspace, id) = fixture(text);
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

    assert_eq!(snapshot.lines[0].source_start, 0);
    assert_eq!(snapshot.lines[0].source_end, text.find('\n').unwrap());
    let after_first_line = text.find('\n').unwrap() + 1;
    let second_statement_start =
        after_first_line + text[after_first_line..].find("SELECT").unwrap();
    assert_eq!(snapshot.lines[2].source_start, second_statement_start);
    assert_eq!(snapshot.lines[2].source_end, text.len());
    let ignore_col_start = text.find("ignore_col").unwrap();
    assert!(snapshot.lines[2].spans.iter().any(|span| {
        span.source_start <= ignore_col_start
            && span.source_end >= ignore_col_start + "ignore_col".len()
    }));
}

#[test]
fn render_snapshot_keeps_projection_mapping_whole_line_with_tabs() {
    let (workspace, id) = fixture("SELECT\tignore_col\n\nSELECT ok");
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
    let line = &snapshot.lines[0];

    assert_eq!(line.display_text, "SELECT  ignore_col");
    assert_eq!(
        line.spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>(),
        line.display_text
    );
    assert_eq!(line.source_start, 0);
    assert_eq!(line.source_end, "SELECT\tignore_col".len());
    assert_eq!(
        line.source_to_display_bytes.last(),
        Some(&line.display_text.len())
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
fn normal_caret_motion_does_not_create_a_selection() {
    let (mut workspace, id) = normal_fixture("  select 1;\nselect 2;");

    press_keys(&mut workspace, id, "$^");

    assert_eq!(
        workspace.position(id).unwrap(),
        EditorPosition { line: 0, column: 2 }
    );
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::Normal);

    let snapshot = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Generic,
        )
        .unwrap();
    assert!(snapshot.selections.is_empty());
    assert!(snapshot.selection_cells.is_empty());
    assert!(snapshot.lines.iter().all(|line| !line.selection_newline));
}

#[test]
fn format_range_preserves_visual_selection() {
    let (mut workspace, id) = normal_fixture("select id from users where id = 1;");
    press_keys(&mut workspace, id, "v$");
    let before = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Postgres,
        )
        .unwrap();
    let scope = workspace
        .current_scope(id, crate::sql::SqlDialect::Postgres)
        .unwrap()
        .unwrap();
    let formatted = crate::sql::format_sql(&scope.sql, crate::sql::SqlDialect::Postgres).unwrap();
    let crate::sql::ScopeSource::Contiguous(range) = scope.source else {
        panic!("expected contiguous visual range");
    };
    workspace
        .replace_range_preserving_selection(id, range, &formatted)
        .unwrap();
    let after = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Postgres,
        )
        .unwrap();

    assert_eq!(before.mode, EditorMode::VisualChar);
    assert_eq!(after.mode, EditorMode::VisualChar);
    assert_eq!(after.selections.len(), 1);
    assert_eq!(after.selections[0].shape, EditorSelectionShape::Char);
    assert!(!after.selection_cells.is_empty());
    assert_eq!(
        after.selections[0].start,
        EditorPosition { line: 0, column: 0 }
    );
    assert_eq!(
        after.selections[0].end,
        super::byte_to_char_position(&formatted, formatted.len())
    );
}

#[test]
fn format_range_preserves_reverse_visual_selection() {
    let (mut workspace, id) = normal_fixture("select id from users where id = 1;");
    press_keys(&mut workspace, id, "$vhh");
    let scope = workspace
        .current_scope(id, crate::sql::SqlDialect::Postgres)
        .unwrap()
        .unwrap();
    let formatted = crate::sql::format_sql(&scope.sql, crate::sql::SqlDialect::Postgres).unwrap();
    let crate::sql::ScopeSource::Contiguous(range) = scope.source else {
        panic!("expected contiguous visual range");
    };
    workspace
        .replace_range_preserving_selection(id, range, &formatted)
        .unwrap();
    let after = workspace
        .render_snapshot_with_dialect(
            id,
            EditorViewport {
                width: 80,
                height: 10,
            },
            crate::sql::SqlDialect::Postgres,
        )
        .unwrap();

    assert_eq!(after.mode, EditorMode::VisualChar);
    assert_eq!(after.selections.len(), 1);
    assert!(!after.selection_cells.is_empty());
    let cursor = workspace.position(id).unwrap();
    assert!(
        (
            after.selections[0].start.line,
            after.selections[0].start.column
        ) <= (cursor.line, cursor.column)
            && (cursor.line, cursor.column)
                <= (after.selections[0].end.line, after.selections[0].end.column)
    );
}

#[test]
fn normal_indent_shortcuts_change_the_current_line() {
    let (mut workspace, id) = normal_fixture("select 1;\nselect 2;");

    press_keys(&mut workspace, id, ">>");
    assert_eq!(workspace.text(id).unwrap(), "    select 1;\nselect 2;");

    press_keys(&mut workspace, id, "<<");
    assert_eq!(workspace.text(id).unwrap(), "select 1;\nselect 2;");
}

#[test]
fn visual_indent_shortcuts_change_selected_lines() {
    let (mut workspace, id) = normal_fixture("select 1;\nselect 2;\nselect 3;");

    press_keys(&mut workspace, id, "Vj>");
    assert_eq!(
        workspace.text(id).unwrap(),
        "    select 1;\n    select 2;\nselect 3;"
    );
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualLine);
}

#[test]
fn counted_normal_indent_affects_multiple_lines() {
    let (mut workspace, id) = normal_fixture("select 1;\nselect 2;\nselect 3;");

    press_keys(&mut workspace, id, "2>>");

    assert_eq!(
        workspace.text(id).unwrap(),
        "    select 1;\n    select 2;\nselect 3;"
    );
}

#[test]
fn visual_block_indent_preserves_block_mode() {
    let (mut workspace, id) = normal_fixture("select 1;\nselect 2;");

    workspace.press(id, EditorKey::Control('v')).unwrap();
    press_keys(&mut workspace, id, "j>");

    assert_eq!(workspace.text(id).unwrap(), "    select 1;\n    select 2;");
    assert_eq!(workspace.mode(id).unwrap(), EditorMode::VisualBlock);
}

#[test]
fn read_only_and_insert_modes_do_not_indent_with_vim_shortcuts() {
    let (mut read_only, read_only_id) = read_only_fixture("select 1;");
    press_keys(&mut read_only, read_only_id, ">>");
    assert_eq!(read_only.text(read_only_id).unwrap(), "select 1;");

    let (mut workspace, id) = fixture("select 1;");
    press_keys(&mut workspace, id, ">>");
    assert_eq!(workspace.text(id).unwrap(), ">>select 1;");
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
fn search_positions_share_the_jump_list_with_vim_motions() {
    let (mut workspace, id) = normal_fixture("one\ntwo\none");
    workspace.press(id, EditorKey::Character('G')).unwrap();
    workspace.press(id, EditorKey::Character('/')).unwrap();
    press_keys(&mut workspace, id, "one");
    workspace.press(id, EditorKey::Enter).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 0);

    workspace.press(id, EditorKey::Control('o')).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 2);
    workspace.press(id, EditorKey::Control('i')).unwrap();
    assert_eq!(workspace.position(id).unwrap().line, 0);
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
#[test]
fn wrapped_preview_scrolls_visual_rows_and_preserves_source_positions() {
    let mut workspace = EditorWorkspace::new();
    let id = uuid::Uuid::new_v4();
    workspace.open_read_only(id, "ab界cd界efghijkl");
    let viewport = EditorViewport {
        width: 5,
        height: 2,
    };
    let language = crate::model::editor_language::EditorLanguage::Plain;
    let first = workspace
        .render_wrapped_preview_snapshot(id, viewport, language, true)
        .unwrap();
    assert_eq!(first.lines[1].wrap_offset, 5);
    assert_eq!(first.lines[1].line, 0);
    assert_eq!(first.total_lines, 4);
    workspace.scroll(id, 1, 0).unwrap();
    let second = workspace
        .render_wrapped_preview_snapshot(id, viewport, language, true)
        .unwrap();
    assert_eq!(second.first_line, 1);
    assert_eq!(second.lines[0].wrap_offset, 5);
    assert_eq!(workspace.text(id).unwrap(), "ab界cd界efghijkl");
    workspace.set_scroll_axis(id, true, usize::MAX).unwrap();
    assert_eq!(
        workspace
            .render_wrapped_preview_snapshot(id, viewport, language, true)
            .unwrap()
            .first_line,
        2
    );
    let plain = workspace
        .render_wrapped_preview_snapshot(id, viewport, language, false)
        .unwrap();
    assert_eq!(plain.total_lines, 1);
    assert_eq!(plain.lines[0].wrap_offset, 0);
}

#[test]
fn wrapped_preview_page_keys_move_cursor_and_keep_it_visible() {
    let text = (0..12)
        .map(|line| format!("line-{line}-abcdefgh"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut workspace = EditorWorkspace::new();
    let id = uuid::Uuid::new_v4();
    workspace.open_read_only(id, &text);
    let viewport = EditorViewport {
        width: 6,
        height: 3,
    };
    let language = crate::model::editor_language::EditorLanguage::Plain;
    workspace
        .render_wrapped_preview_snapshot(id, viewport, language, true)
        .unwrap();
    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('d'),
                crossterm::event::KeyModifiers::CONTROL,
            ),
        )
        .unwrap();
    let after_half_page = workspace.position(id).unwrap();
    assert!(after_half_page.line > 0);
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, language, true)
        .unwrap();
    assert!(snapshot.cursor_screen_cell.is_some());

    workspace
        .key(
            id,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::PageDown,
                crossterm::event::KeyModifiers::NONE,
            ),
        )
        .unwrap();
    let after_page = workspace.position(id).unwrap();
    assert!(after_page.line >= after_half_page.line);
    let snapshot = workspace
        .render_wrapped_preview_snapshot(id, viewport, language, true)
        .unwrap();
    assert!(snapshot.cursor_screen_cell.is_some());
    assert_eq!(workspace.text(id).unwrap(), text);
}

#[test]
fn preview_yaml_comments_are_not_duplicated_and_strings_keep_hashes() {
    let spans = EditorWorkspace::preview_highlight_spans(
        "name: 'a#b' # comment",
        0,
        crate::model::editor_language::EditorLanguage::Yaml,
    );
    assert_eq!(
        spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>(),
        "name: 'a#b' # comment"
    );
    assert!(
        spans
            .iter()
            .any(|span| span.text == "'a#b'" && span.kind == EditorHighlightKind::String)
    );
    assert_eq!(spans.last().unwrap().kind, EditorHighlightKind::Comment);
}
