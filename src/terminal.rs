use std::{
    io::{self, Stdout},
    panic,
    sync::atomic::{AtomicBool, Ordering},
};

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static KEYBOARD_ENHANCEMENT_ENABLED: AtomicBool = AtomicBool::new(false);

pub struct TerminalSession {
    terminal: Tui,
    mouse_enabled: bool,
    mouse_captured: bool,
}

impl TerminalSession {
    pub fn enter(mouse_enabled: bool) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        ) {
            restore_terminal();
            return Err(error);
        }
        if supports_keyboard_enhancement().unwrap_or(false) {
            if let Err(error) = execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            ) {
                restore_terminal();
                return Err(error);
            }
            KEYBOARD_ENHANCEMENT_ENABLED.store(true, Ordering::Relaxed);
        }
        if mouse_enabled && let Err(error) = execute!(stdout, EnableMouseCapture) {
            restore_terminal();
            return Err(error);
        }
        let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_terminal();
                return Err(error);
            }
        };
        if let Err(error) = terminal.clear().and_then(|_| terminal.hide_cursor()) {
            restore_terminal();
            return Err(error);
        }
        Ok(Self {
            terminal,
            mouse_enabled,
            mouse_captured: mouse_enabled,
        })
    }

    pub fn mouse_captured(&self) -> bool {
        self.mouse_captured
    }

    pub fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        if enabled == self.mouse_captured {
            return Ok(());
        }
        if enabled {
            execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
        } else {
            execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
        }
        self.mouse_captured = enabled;
        Ok(())
    }

    pub fn draw<F>(&mut self, render: F) -> io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render).map(|_| ())
    }

    pub fn set_cursor_style(&mut self, style: crate::ui::CursorStyle) -> io::Result<()> {
        let style = match style {
            crate::ui::CursorStyle::Block => SetCursorStyle::SteadyBlock,
            crate::ui::CursorStyle::Bar => SetCursorStyle::SteadyBar,
            crate::ui::CursorStyle::Underline => SetCursorStyle::SteadyUnderScore,
        };
        execute!(self.terminal.backend_mut(), style)
    }

    pub fn write_osc52(&mut self, text: &str, max_bytes: usize) -> io::Result<()> {
        write_osc52_to(self.terminal.backend_mut(), text, max_bytes)
    }
}

fn write_osc52_to(writer: &mut impl io::Write, text: &str, max_bytes: usize) -> io::Result<()> {
    let sequence = crate::clipboard::osc52_sequence(text, max_bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    writer.write_all(sequence.as_bytes())?;
    writer.flush()
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let backend = self.terminal.backend_mut();
        if self.mouse_enabled {
            let _ = execute!(backend, DisableMouseCapture);
        }
        disable_keyboard_enhancement(backend);
        let _ = execute!(
            backend,
            DisableFocusChange,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
        let _ = execute!(
            self.terminal.backend_mut(),
            SetCursorStyle::DefaultUserShape
        );
    }
}

pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

pub fn restore_terminal() {
    let mut stdout = io::stdout();
    disable_keyboard_enhancement(&mut stdout);
    let _ = execute!(
        stdout,
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

fn disable_keyboard_enhancement(writer: &mut impl io::Write) {
    if KEYBOARD_ENHANCEMENT_ENABLED.swap(false, Ordering::Relaxed) {
        let _ = execute!(writer, PopKeyboardEnhancementFlags);
    }
}

#[cfg(test)]
mod tests {
    use super::write_osc52_to;

    #[test]
    fn osc52_is_written_as_one_flushed_terminal_sequence() {
        let mut output = Vec::new();
        write_osc52_to(&mut output, "a\n你", 100).unwrap();
        assert_eq!(output, b"\x1b]52;c;YQrkvaA=\x07");
    }

    #[test]
    fn osc52_size_errors_are_not_written() {
        let mut output = Vec::new();
        assert!(write_osc52_to(&mut output, "hello", 4).is_err());
        assert!(output.is_empty());
    }
}
