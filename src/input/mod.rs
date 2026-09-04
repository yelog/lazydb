use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub mod keymap;
pub mod mouse;

fn text_history_modifier() -> KeyModifiers {
    if cfg!(target_os = "macos") {
        KeyModifiers::SUPER
    } else {
        KeyModifiers::CONTROL
    }
}

fn is_text_undo_with_modifier(event: KeyEvent, modifier: KeyModifiers) -> bool {
    event.modifiers == modifier && event.code == KeyCode::Char('z')
}

fn is_text_redo_with_modifier(event: KeyEvent, modifier: KeyModifiers) -> bool {
    (event.modifiers == (modifier | KeyModifiers::SHIFT)
        && matches!(event.code, KeyCode::Char('z' | 'Z')))
        || (event.modifiers == modifier && event.code == KeyCode::Char('Z'))
}

pub(crate) fn is_text_undo(event: KeyEvent) -> bool {
    is_text_undo_with_modifier(event, text_history_modifier())
}

pub(crate) fn is_text_redo(event: KeyEvent) -> bool {
    is_text_redo_with_modifier(event, text_history_modifier())
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        is_text_redo, is_text_redo_with_modifier, is_text_undo, is_text_undo_with_modifier,
    };

    #[test]
    fn text_undo_uses_the_requested_primary_modifier() {
        for modifier in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
            assert!(is_text_undo_with_modifier(
                KeyEvent::new(KeyCode::Char('z'), modifier),
                modifier,
            ));
            assert!(!is_text_undo_with_modifier(
                KeyEvent::new(KeyCode::Char('z'), modifier | KeyModifiers::ALT),
                modifier,
            ));
            assert!(!is_text_undo_with_modifier(
                KeyEvent::new(KeyCode::Char('Z'), modifier),
                modifier,
            ));
        }
    }

    #[test]
    fn text_redo_accepts_supported_terminal_encodings() {
        for modifier in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
            for event in [
                KeyEvent::new(KeyCode::Char('z'), modifier | KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('Z'), modifier),
                KeyEvent::new(KeyCode::Char('Z'), modifier | KeyModifiers::SHIFT),
            ] {
                assert!(is_text_redo_with_modifier(event, modifier));
            }
        }
    }

    #[test]
    fn text_history_rejects_the_wrong_or_extra_modifier() {
        assert!(!is_text_undo_with_modifier(
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::SUPER),
            KeyModifiers::CONTROL,
        ));
        assert!(!is_text_redo_with_modifier(
            KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT,
            ),
            KeyModifiers::CONTROL,
        ));
    }

    #[test]
    fn text_history_uses_the_platform_primary_modifier() {
        let expected = if cfg!(target_os = "macos") {
            KeyModifiers::SUPER
        } else {
            KeyModifiers::CONTROL
        };
        let unexpected = if cfg!(target_os = "macos") {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::SUPER
        };

        assert!(is_text_undo(KeyEvent::new(KeyCode::Char('z'), expected,)));
        assert!(is_text_redo(KeyEvent::new(
            KeyCode::Char('z'),
            expected | KeyModifiers::SHIFT,
        )));
        assert!(!is_text_undo(
            KeyEvent::new(KeyCode::Char('z'), unexpected,)
        ));
    }
}
